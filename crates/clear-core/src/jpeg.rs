//! JPEG: segment-level parsing and rewriting. The scan data is never decoded.

use crate::{be16, malformed, Block, CleanOptions, Fnv, Kind, Parsed, Result};

enum Seg<'a> {
    /// A marker segment with a length field: (marker, whole segment bytes incl. FFxx and length, payload)
    Marker(u8, &'a [u8], &'a [u8]),
    /// SOS segment plus entropy-coded data up to the next non-RST marker.
    Scan(&'a [u8]),
    /// Bytes after EOI.
    Trailer(&'a [u8]),
}

fn segments(d: &[u8]) -> Result<Vec<Seg<'_>>> {
    let mut out = Vec::new();
    let mut i = 2; // after SOI
    loop {
        if i >= d.len() {
            return Err(malformed("missing end-of-image marker"));
        }
        if d[i] != 0xFF {
            return Err(malformed("expected a marker"));
        }
        // Skip fill bytes.
        while i < d.len() && d[i] == 0xFF {
            i += 1;
        }
        let m = *d.get(i).ok_or_else(|| malformed("truncated marker"))?;
        let start = i - 1;
        i += 1;
        match m {
            0xD9 => {
                out.push(Seg::Marker(0xD9, &d[start..i], &[]));
                if i < d.len() {
                    out.push(Seg::Trailer(&d[i..]));
                }
                return Ok(out);
            }
            0x01 | 0xD0..=0xD7 => out.push(Seg::Marker(m, &d[start..i], &[])),
            _ => {
                let len = be16(d, i).ok_or_else(|| malformed("truncated segment length"))? as usize;
                if len < 2 || i + len > d.len() {
                    return Err(malformed("segment length out of range"));
                }
                let end = i + len;
                if m == 0xDA {
                    // Entropy-coded data: runs until a marker other than RSTn / stuffed 0x00.
                    let mut j = end;
                    while j + 1 < d.len() {
                        if d[j] == 0xFF && d[j + 1] != 0x00 && !(0xD0..=0xD7).contains(&d[j + 1]) {
                            break;
                        }
                        j += 1;
                    }
                    if j + 1 >= d.len() {
                        return Err(malformed("scan data runs past the end of the file"));
                    }
                    out.push(Seg::Scan(&d[start..j]));
                    i = j;
                } else {
                    out.push(Seg::Marker(m, &d[start..end], &d[i + 2..end]));
                    i = end;
                }
            }
        }
    }
}

fn classify(m: u8, p: &[u8]) -> Option<(Kind, String)> {
    match m {
        0xE1 if p.starts_with(b"Exif\0") => Some((Kind::Exif, "APP1 Exif".into())),
        0xE1 if p.starts_with(b"http://ns.adobe.com/xap/1.0/\0") => {
            Some((Kind::Xmp, "APP1 XMP".into()))
        }
        0xE1 if p.starts_with(b"http://ns.adobe.com/xmp/extension/\0") => {
            Some((Kind::Xmp, "APP1 Extended XMP".into()))
        }
        0xE2 if p.starts_with(b"ICC_PROFILE\0") => Some((Kind::Icc, "APP2 ICC profile".into())),
        0xEB if p.len() >= 16 && p.starts_with(b"JP") && contains(p, b"c2pa") => {
            Some((Kind::C2pa, "APP11 C2PA manifest".into()))
        }
        0xED if p.starts_with(b"Photoshop 3.0\0") => {
            Some((Kind::Iptc, "APP13 IPTC / Photoshop".into()))
        }
        0xFE => Some((Kind::Comment, "Comment".into())),
        // JFIF (APP0) and Adobe colour transform (APP14) are structural and kept.
        0xE0 | 0xEE => None,
        0xE1..=0xEF => Some((Kind::Other, format!("APP{} {}", m - 0xE0, ident(p)))),
        _ => None,
    }
}

fn ident(p: &[u8]) -> String {
    let s: String = p
        .iter()
        .take(24)
        .take_while(|&&b| b != 0)
        .filter(|b| b.is_ascii_graphic() || **b == b' ')
        .map(|&b| b as char)
        .collect();
    if s.is_empty() {
        "data".into()
    } else {
        s
    }
}

pub(crate) fn contains(h: &[u8], n: &[u8]) -> bool {
    h.windows(n.len()).any(|w| w == n)
}

pub fn parse(d: &[u8]) -> Result<Parsed> {
    let segs = segments(d)?;
    let mut p = Parsed {
        width: 0,
        height: 0,
        has_alpha: false,
        animated: false,
        blocks: Vec::new(),
        exif_tiff: None,
        fingerprint: 0,
    };
    let mut fp = Fnv::new();
    for s in &segs {
        match s {
            Seg::Marker(m, whole, payload) => {
                if let Some((kind, label)) = classify(*m, payload) {
                    if kind == Kind::Exif && p.exif_tiff.is_none() {
                        p.exif_tiff = Some(payload[6..].to_vec());
                    }
                    p.blocks.push(Block {
                        kind,
                        label,
                        size: whole.len(),
                    });
                } else {
                    if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)
                        && payload.len() >= 5
                    {
                        p.height = be16(payload, 1).unwrap_or(0) as u32;
                        p.width = be16(payload, 3).unwrap_or(0) as u32;
                    }
                    if *m != 0xE0 {
                        fp.add(whole);
                    }
                }
            }
            Seg::Scan(bytes) => fp.add(bytes),
            Seg::Trailer(t) => {
                if t.iter().any(|&b| b != 0) {
                    p.blocks.push(Block {
                        kind: Kind::Other,
                        label: "Data after end of image".into(),
                        size: t.len(),
                    });
                }
            }
        }
    }
    if p.width == 0 || p.height == 0 {
        return Err(malformed("no frame header"));
    }
    p.fingerprint = fp.finish();
    Ok(p)
}

pub fn clean(d: &[u8], o: &CleanOptions, minimal_exif: Option<&[u8]>) -> Result<Vec<u8>> {
    let segs = segments(d)?;
    let mut out = Vec::with_capacity(d.len());
    out.extend_from_slice(&[0xFF, 0xD8]);
    let mut exif_written = minimal_exif.is_none();
    let write_exif = |out: &mut Vec<u8>, tiff: &[u8]| {
        let len = (2 + 6 + tiff.len()) as u16;
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(b"Exif\0\0");
        out.extend_from_slice(tiff);
    };
    for s in &segs {
        match s {
            Seg::Marker(m, whole, payload) => {
                if !exif_written && *m != 0xE0 {
                    write_exif(&mut out, minimal_exif.unwrap());
                    exif_written = true;
                }
                match classify(*m, payload) {
                    Some((kind, _)) if o.removes(kind) => {}
                    _ => out.extend_from_slice(whole),
                }
            }
            Seg::Scan(bytes) => out.extend_from_slice(bytes),
            Seg::Trailer(t) => {
                if !o.other {
                    out.extend_from_slice(t);
                }
            }
        }
    }
    Ok(out)
}
