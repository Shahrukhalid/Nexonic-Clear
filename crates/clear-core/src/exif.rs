//! Minimal, bounds-checked EXIF (TIFF) reader for the fields people care about,
//! and a writer for an orientation-only EXIF block.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExifSummary {
    /// Readable fields in display order, e.g. ("Camera", "Apple iPhone 15").
    pub fields: Vec<(String, String)>,
    pub has_gps: bool,
    pub has_thumbnail: bool,
    pub tag_count: usize,
}

/// IFD entry: tag, type, count, position of the value or its offset.
type Entry = (u16, u16, u32, usize);

struct Tiff<'a> {
    d: &'a [u8],
    le: bool,
}

impl<'a> Tiff<'a> {
    fn new(d: &'a [u8]) -> Option<Self> {
        let le = match d.get(0..4)? {
            b"II*\0" => true,
            b"MM\0*" => false,
            _ => return None,
        };
        Some(Tiff { d, le })
    }
    fn u16(&self, i: usize) -> Option<u16> {
        let b: [u8; 2] = self.d.get(i..i + 2)?.try_into().ok()?;
        Some(if self.le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    }
    fn u32(&self, i: usize) -> Option<u32> {
        let b: [u8; 4] = self.d.get(i..i + 4)?.try_into().ok()?;
        Some(if self.le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }
    /// Entries of the IFD at `off`: (tag, type, count, value-or-offset position) and next IFD offset.
    fn ifd(&self, off: usize) -> Option<(Vec<Entry>, u32)> {
        let n = self.u16(off)? as usize;
        if n > 1000 {
            return None;
        }
        let mut v = Vec::with_capacity(n);
        for k in 0..n {
            let e = off + 2 + k * 12;
            v.push((self.u16(e)?, self.u16(e + 2)?, self.u32(e + 4)?, e + 8));
        }
        let next = self.u32(off + 2 + n * 12).unwrap_or(0);
        Some((v, next))
    }
    fn data(&self, ty: u16, count: u32, pos: usize) -> Option<&'a [u8]> {
        let unit: usize = match ty {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => return None,
        };
        let len = unit.checked_mul(count as usize)?;
        if len > 1 << 20 {
            return None;
        }
        let start = if len <= 4 {
            pos
        } else {
            self.u32(pos)? as usize
        };
        self.d.get(start..start.checked_add(len)?)
    }
    fn text(&self, ty: u16, count: u32, pos: usize) -> Option<String> {
        let b = self.data(ty, count, pos)?;
        let s: String = String::from_utf8_lossy(b)
            .trim_end_matches('\0')
            .trim()
            .chars()
            .take(120)
            .collect();
        (!s.is_empty()).then_some(s)
    }
    fn short(&self, ty: u16, pos: usize) -> Option<u32> {
        match ty {
            3 => self.u16(pos).map(u32::from),
            4 => self.u32(pos),
            _ => None,
        }
    }
    fn rationals(&self, ty: u16, count: u32, pos: usize) -> Option<Vec<f64>> {
        if ty != 5 {
            return None;
        }
        let b = self.data(ty, count, pos)?;
        let t = Tiff { d: b, le: self.le };
        (0..count as usize)
            .map(|k| {
                let (n, d) = (t.u32(k * 8)?, t.u32(k * 8 + 4)?);
                (d != 0).then(|| n as f64 / d as f64)
            })
            .collect()
    }
}

pub fn orientation(tiff: &[u8]) -> Option<u16> {
    let t = Tiff::new(tiff)?;
    let (entries, _) = t.ifd(t.u32(4)? as usize)?;
    entries
        .iter()
        .find(|e| e.0 == 0x0112)
        .and_then(|&(_, ty, _, pos)| t.short(ty, pos))
        .map(|v| v as u16)
        .filter(|v| (1..=8).contains(v))
}

pub fn summarize(tiff: &[u8]) -> Option<ExifSummary> {
    let t = Tiff::new(tiff)?;
    let mut s = ExifSummary::default();
    let ifd0 = t.u32(4)? as usize;
    let (e0, next) = t.ifd(ifd0)?;
    s.tag_count += e0.len();
    s.has_thumbnail = next != 0 && (next as usize) < tiff.len();
    let mut make = None;
    let mut model = None;
    let push = |s: &mut ExifSummary, k: &str, v: Option<String>| {
        if let Some(v) = v {
            s.fields.push((k.into(), v));
        }
    };
    let mut exif_ptr = None;
    let mut gps_ptr = None;
    for &(tag, ty, count, pos) in &e0 {
        match tag {
            0x010F => make = t.text(ty, count, pos),
            0x0110 => model = t.text(ty, count, pos),
            0x0112 => push(
                &mut s,
                "Orientation",
                t.short(ty, pos).map(|v| v.to_string()),
            ),
            0x0131 => push(&mut s, "Software", t.text(ty, count, pos)),
            0x0132 => push(&mut s, "Modified", t.text(ty, count, pos)),
            0x013B => push(&mut s, "Artist", t.text(ty, count, pos)),
            0x8298 => push(&mut s, "Copyright", t.text(ty, count, pos)),
            0x8769 => exif_ptr = t.short(ty, pos),
            0x8825 => gps_ptr = t.short(ty, pos),
            _ => {}
        }
    }
    let camera = match (make, model) {
        (Some(a), Some(b)) if b.starts_with(&a) => Some(b),
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (a, b) => a.or(b),
    };
    if let Some(c) = camera {
        s.fields.insert(0, ("Camera".into(), c));
    }
    if let Some((e, _)) = exif_ptr.and_then(|p| t.ifd(p as usize)) {
        s.tag_count += e.len();
        for &(tag, ty, count, pos) in &e {
            match tag {
                0x9003 => push(&mut s, "Taken", t.text(ty, count, pos)),
                0xA430 => push(&mut s, "Owner", t.text(ty, count, pos)),
                0xA431 => push(&mut s, "Camera serial", t.text(ty, count, pos)),
                0xA434 => push(&mut s, "Lens", t.text(ty, count, pos)),
                0xA435 => push(&mut s, "Lens serial", t.text(ty, count, pos)),
                _ => {}
            }
        }
    }
    if let Some((g, _)) = gps_ptr.and_then(|p| t.ifd(p as usize)) {
        s.tag_count += g.len();
        let get = |tag: u16| g.iter().find(|e| e.0 == tag).copied();
        let coord = |val: u16, refr: u16| -> Option<f64> {
            let (_, ty, count, pos) = get(val)?;
            let r = t.rationals(ty, count, pos)?;
            let deg =
                r.first()? + r.get(1).unwrap_or(&0.0) / 60.0 + r.get(2).unwrap_or(&0.0) / 3600.0;
            let (_, rty, rcount, rpos) = get(refr)?;
            let sign = match t.text(rty, rcount, rpos)?.as_str() {
                "S" | "W" => -1.0,
                _ => 1.0,
            };
            Some(sign * deg)
        };
        s.has_gps = !g.is_empty();
        match (coord(2, 1), coord(4, 3)) {
            (Some(lat), Some(lon)) => push(&mut s, "Location", Some(format!("{lat:.5}, {lon:.5}"))),
            _ if s.has_gps => push(&mut s, "Location", Some("GPS data present".into())),
            _ => {}
        }
    }
    Some(s)
}

/// A big-endian TIFF block containing only IFD0 with one Orientation entry.
pub fn minimal_with_orientation(orientation: u16) -> Vec<u8> {
    let mut v = Vec::with_capacity(26);
    v.extend_from_slice(b"MM\0*");
    v.extend_from_slice(&8u32.to_be_bytes());
    v.extend_from_slice(&1u16.to_be_bytes());
    v.extend_from_slice(&0x0112u16.to_be_bytes());
    v.extend_from_slice(&3u16.to_be_bytes());
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(&orientation.to_be_bytes());
    v.extend_from_slice(&[0, 0]);
    v.extend_from_slice(&0u32.to_be_bytes());
    v
}
