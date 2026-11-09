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

/// One readable metadata field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Field {
    /// "EXIF", "GPS", "Thumbnail", "XMP", "IPTC", "Text", "Comment", "ICC".
    pub group: String,
    pub name: String,
    pub value: String,
    /// EXIF tag number, when the field comes from EXIF.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<u16>,
}

fn tag_name(group: &str, tag: u16) -> Option<&'static str> {
    Some(match (group, tag) {
        ("GPS", 0x0000) => "GPS version",
        ("GPS", 0x0001) => "Latitude reference",
        ("GPS", 0x0002) => "Latitude",
        ("GPS", 0x0003) => "Longitude reference",
        ("GPS", 0x0004) => "Longitude",
        ("GPS", 0x0005) => "Altitude reference",
        ("GPS", 0x0006) => "Altitude",
        ("GPS", 0x0007) => "GPS time",
        ("GPS", 0x0010) => "Image direction reference",
        ("GPS", 0x0011) => "Image direction",
        ("GPS", 0x0012) => "Map datum",
        ("GPS", 0x001B) => "Processing method",
        ("GPS", 0x001D) => "GPS date",
        ("GPS", _) => return None,
        (_, 0x010E) => "Image description",
        (_, 0x010F) => "Camera make",
        (_, 0x0110) => "Camera model",
        (_, 0x0112) => "Orientation",
        (_, 0x011A) => "X resolution",
        (_, 0x011B) => "Y resolution",
        (_, 0x0128) => "Resolution unit",
        (_, 0x0131) => "Software",
        (_, 0x0132) => "Date modified",
        (_, 0x013B) => "Artist",
        (_, 0x013C) => "Host computer",
        (_, 0x0213) => "YCbCr positioning",
        (_, 0x0103) => "Compression",
        (_, 0x0201) => "Thumbnail offset",
        (_, 0x0202) => "Thumbnail length",
        (_, 0x8298) => "Copyright",
        (_, 0x829A) => "Exposure time",
        (_, 0x829D) => "F-number",
        (_, 0x8822) => "Exposure program",
        (_, 0x8827) => "ISO",
        (_, 0x8830) => "Sensitivity type",
        (_, 0x8831) => "Standard output sensitivity",
        (_, 0x8832) => "Recommended exposure index",
        (_, 0x9101) => "Components configuration",
        (_, 0x9102) => "Compressed bits per pixel",
        (_, 0x9203) => "Brightness",
        (_, 0x9205) => "Maximum aperture",
        (_, 0x9206) => "Subject distance",
        (_, 0x9208) => "Light source",
        (_, 0x9214) => "Subject area",
        (_, 0xA000) => "FlashPix version",
        (_, 0xA20E) => "Focal plane X resolution",
        (_, 0xA20F) => "Focal plane Y resolution",
        (_, 0xA210) => "Focal plane resolution unit",
        (_, 0xA217) => "Sensing method",
        (_, 0xA300) => "File source",
        (_, 0xA401) => "Custom rendered",
        (_, 0xA404) => "Digital zoom ratio",
        (_, 0xA406) => "Scene capture type",
        (_, 0xA408) => "Contrast",
        (_, 0xA409) => "Saturation",
        (_, 0xA40A) => "Sharpness",
        (_, 0xA40C) => "Subject distance range",
        (_, 0x9000) => "EXIF version",
        (_, 0x9003) => "Date taken",
        (_, 0x9004) => "Date digitized",
        (_, 0x9010) => "Time zone",
        (_, 0x9011) => "Time zone (taken)",
        (_, 0x9012) => "Time zone (digitized)",
        (_, 0x9201) => "Shutter speed",
        (_, 0x9202) => "Aperture",
        (_, 0x9204) => "Exposure compensation",
        (_, 0x9207) => "Metering mode",
        (_, 0x9209) => "Flash",
        (_, 0x920A) => "Focal length",
        (_, 0x927C) => "Maker notes",
        (_, 0x9286) => "User comment",
        (_, 0x9290) => "Subsecond time",
        (_, 0x9291) => "Subsecond time (taken)",
        (_, 0x9292) => "Subsecond time (digitized)",
        (_, 0xA001) => "Colour space",
        (_, 0xA002) => "Pixel width",
        (_, 0xA003) => "Pixel height",
        (_, 0xA402) => "Exposure mode",
        (_, 0xA403) => "White balance",
        (_, 0xA405) => "Focal length (35 mm)",
        (_, 0xA420) => "Unique image ID",
        (_, 0xA430) => "Camera owner",
        (_, 0xA431) => "Camera serial number",
        (_, 0xA432) => "Lens specification",
        (_, 0xA433) => "Lens make",
        (_, 0xA434) => "Lens model",
        (_, 0xA435) => "Lens serial number",
        (_, 0xA460) => "Composite image",
        _ => return None,
    })
}

impl Tiff<'_> {
    fn format_value(&self, ty: u16, count: u32, pos: usize) -> String {
        match ty {
            2 => self.text(ty, count, pos).unwrap_or_default(),
            3 | 4 | 8 | 9 => {
                let unit = if matches!(ty, 3 | 8) { 2 } else { 4 };
                let n = (count as usize).min(8);
                let Some(b) = self.data(ty, count, pos) else {
                    return String::new();
                };
                let t = Tiff { d: b, le: self.le };
                let vals: Vec<String> = (0..n)
                    .filter_map(|k| {
                        let v = if unit == 2 {
                            t.u16(k * 2).map(|v| v as i64)
                        } else {
                            t.u32(k * 4).map(|v| v as i64)
                        };
                        v.map(|v| match ty {
                            8 => (v as i16).to_string(),
                            9 => (v as i32).to_string(),
                            _ => v.to_string(),
                        })
                    })
                    .collect();
                let more = if count as usize > n { " …" } else { "" };
                format!("{}{more}", vals.join(", "))
            }
            5 | 10 => {
                let Some(b) = self.data(ty, count, pos) else {
                    return String::new();
                };
                let t = Tiff { d: b, le: self.le };
                let vals: Vec<String> = (0..(count as usize).min(8))
                    .filter_map(|k| {
                        let (n, d) = (t.u32(k * 8)?, t.u32(k * 8 + 4)?);
                        let (n, d) = if ty == 10 {
                            (n as i32 as f64, d as i32 as f64)
                        } else {
                            (n as f64, d as f64)
                        };
                        Some(if d == 0.0 {
                            "–".into()
                        } else {
                            trim_float(n / d)
                        })
                    })
                    .collect();
                vals.join(", ")
            }
            1 | 6 | 7 => {
                let b = self.data(ty, count, pos).unwrap_or_default();
                if b.len() <= 16
                    && !b.is_empty()
                    && b.iter()
                        .all(|c| c.is_ascii_graphic() || *c == b' ' || *c == 0)
                {
                    String::from_utf8_lossy(b)
                        .trim_end_matches('\0')
                        .to_string()
                } else {
                    format!("{} bytes", b.len())
                }
            }
            _ => format!("{count} values"),
        }
    }

    fn collect(&self, off: usize, group: &str, out: &mut Vec<Field>, depth: u8) {
        let Some((entries, _)) = self.ifd(off) else {
            return;
        };
        for &(tag, ty, count, pos) in &entries {
            if out.len() >= 1000 {
                return;
            }
            let child = match (group, tag) {
                (_, 0x8769) => Some("EXIF"),
                (_, 0x8825) => Some("GPS"),
                (_, 0xA005) => Some("Interoperability"),
                _ => None,
            };
            if let Some(g) = child {
                if depth < 3 {
                    if let Some(p) = self.short(ty, pos) {
                        self.collect(p as usize, g, out, depth + 1);
                    }
                }
                continue;
            }
            let name = tag_name(group, tag)
                .map(str::to_string)
                .unwrap_or_else(|| format!("Tag 0x{tag:04X}"));
            let value: String = self
                .format_value(ty, count, pos)
                .chars()
                .take(300)
                .collect();
            out.push(Field {
                group: group.into(),
                name,
                value,
                tag: Some(tag),
            });
        }
    }
}

fn trim_float(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Every EXIF, GPS and thumbnail-IFD field, with readable names where known.
pub fn all_fields(tiff: &[u8]) -> Vec<Field> {
    let Some(t) = Tiff::new(tiff) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Some(ifd0) = t.u32(4) else { return out };
    t.collect(ifd0 as usize, "EXIF", &mut out, 0);
    if let Some((_, next)) = t.ifd(ifd0 as usize) {
        if next != 0 {
            t.collect(next as usize, "Thumbnail", &mut out, 2);
        }
    }
    out
}

/// What to carry over when EXIF is removed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Keep {
    pub orientation: bool,
    pub dates: bool,
    pub resolution: bool,
}

/// Tags written back by [`preserved`], so verification can allow exactly these.
pub fn kept_tags(k: Keep) -> Vec<u16> {
    let mut v = Vec::new();
    if k.orientation {
        v.push(0x0112);
    }
    if k.resolution {
        v.extend([0x011A, 0x011B, 0x0128]);
    }
    if k.dates {
        v.extend([0x0132, 0x9003, 0x9004, 0x9010, 0x9011, 0x9012]);
    }
    v
}

/// A new big-endian EXIF block holding only the selected tags copied from
/// `tiff`. Returns None when there is nothing to keep.
pub fn preserved(tiff: &[u8], k: Keep) -> Option<Vec<u8>> {
    let t = Tiff::new(tiff)?;
    let ifd0 = t.u32(4)? as usize;
    let (e0, _) = t.ifd(ifd0)?;
    let exif_entries = e0
        .iter()
        .find(|e| e.0 == 0x8769)
        .and_then(|&(_, ty, _, pos)| t.short(ty, pos))
        .and_then(|p| t.ifd(p as usize))
        .map(|(e, _)| e)
        .unwrap_or_default();
    // (tag, type, count, big-endian value bytes)
    let pick = |entries: &[Entry], want: &[u16]| -> Vec<(u16, u16, u32, Vec<u8>)> {
        let mut v: Vec<(u16, u16, u32, Vec<u8>)> = Vec::new();
        for &(tag, ty, count, pos) in entries {
            if !want.contains(&tag) || !matches!(ty, 2..=5) || count == 0 || count > 64 {
                continue;
            }
            let Some(raw) = t.data(ty, count, pos) else {
                continue;
            };
            let mut be = Vec::with_capacity(raw.len());
            match ty {
                2 => be.extend_from_slice(raw),
                3 => raw.chunks(2).for_each(|c| {
                    let x = if t.le {
                        u16::from_le_bytes([c[0], c[1]])
                    } else {
                        u16::from_be_bytes([c[0], c[1]])
                    };
                    be.extend_from_slice(&x.to_be_bytes())
                }),
                _ => raw.chunks(4).for_each(|c| {
                    let a = [c[0], c[1], c[2], c[3]];
                    let x = if t.le {
                        u32::from_le_bytes(a)
                    } else {
                        u32::from_be_bytes(a)
                    };
                    be.extend_from_slice(&x.to_be_bytes())
                }),
            }
            if tag == 0x0112 && be != [0, 1] && ty == 3 && count == 1 || tag != 0x0112 {
                v.push((tag, ty, count, be));
            }
        }
        v.sort_by_key(|e| e.0);
        v
    };
    let tags = kept_tags(k);
    let mut ifd0_tags = pick(&e0, &tags);
    let exif_tags = pick(&exif_entries, &tags);
    if ifd0_tags.is_empty() && exif_tags.is_empty() {
        return None;
    }
    // Layout: header(8) | IFD0 | IFD0 data | ExifIFD | ExifIFD data
    let ifd_size = |n: usize| 2 + n * 12 + 4;
    let data_size = |v: &[(u16, u16, u32, Vec<u8>)]| -> usize {
        v.iter()
            .filter(|e| e.3.len() > 4)
            .map(|e| e.3.len() + (e.3.len() & 1))
            .sum()
    };
    if !exif_tags.is_empty() {
        ifd0_tags.push((0x8769, 4, 1, vec![0; 4]));
        ifd0_tags.sort_by_key(|e| e.0);
    }
    let ifd0_at = 8usize;
    let ifd0_data = ifd0_at + ifd_size(ifd0_tags.len());
    let exif_at = ifd0_data + data_size(&ifd0_tags);
    if let Some(p) = ifd0_tags.iter_mut().find(|e| e.0 == 0x8769) {
        p.3 = (exif_at as u32).to_be_bytes().to_vec();
    }
    let mut out = b"MM\0*\0\0\0\x08".to_vec();
    let write_ifd = |out: &mut Vec<u8>, tags: &[(u16, u16, u32, Vec<u8>)], at: usize| {
        let mut data_at = at + ifd_size(tags.len());
        let mut data = Vec::new();
        out.extend_from_slice(&(tags.len() as u16).to_be_bytes());
        for (tag, ty, count, val) in tags {
            out.extend_from_slice(&tag.to_be_bytes());
            out.extend_from_slice(&ty.to_be_bytes());
            out.extend_from_slice(&count.to_be_bytes());
            if val.len() <= 4 {
                let mut v = val.clone();
                v.resize(4, 0);
                out.extend_from_slice(&v);
            } else {
                out.extend_from_slice(&(data_at as u32).to_be_bytes());
                data.extend_from_slice(val);
                if val.len() & 1 == 1 {
                    data.push(0);
                }
                data_at += val.len() + (val.len() & 1);
            }
        }
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&data);
    };
    write_ifd(&mut out, &ifd0_tags, ifd0_at);
    if !exif_tags.is_empty() {
        debug_assert_eq!(out.len(), exif_at);
        write_ifd(&mut out, &exif_tags, exif_at);
    }
    Some(out)
}
