//! JPEG: segment-level parsing and rewriting. The scan data is never decoded.
//!
//! Content after the end-of-image marker is classified: motion-photo video
//! (an MP4 `ftyp` box or Samsung motion data) and further JPEG images (MPF
//! secondary images such as depth maps or Ultra HDR gain maps) are
//! `Kind::Embedded`; anything else non-zero is `Kind::Other`.

use crate::{be16, details, malformed, Block, CleanOptions, Digest, Kind, Parsed, Result};

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
    let mut frames = 0usize;
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
            0xD8 => return Err(malformed("unexpected start-of-image marker")),
            0x00 => return Err(malformed("invalid marker")),
            0x01 | 0xD0..=0xD7 => out.push(Seg::Marker(m, &d[start..i], &[])),
            _ => {
                let len = be16(d, i).ok_or_else(|| malformed("truncated segment length"))? as usize;
                if len < 2 || i + len > d.len() {
                    return Err(malformed("segment length out of range"));
                }
                let end = i + len;
                if is_frame(m) {
                    frames += 1;
                    if frames > 1 {
                        return Err(malformed("more than one frame header"));
                    }
                }
                if m == 0xDA {
                    if frames == 0 {
                        return Err(malformed("scan before frame header"));
                    }
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

fn is_frame(m: u8) -> bool {
    matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)
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
        0xE2 if p.starts_with(b"MPF\0") => {
            Some((Kind::Embedded, "APP2 multi-picture index".into()))
        }
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

/// What the bytes after end-of-image hold.
fn trailer_kind(t: &[u8]) -> Option<(Kind, &'static str)> {
    let head = &t[..t.len().min(64)];
    if contains(head, b"ftyp") || contains(t, b"MotionPhoto_Data") {
        Some((Kind::Embedded, "Motion photo video"))
    } else if t.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some((
            Kind::Embedded,
            "Additional images (depth, HDR gain map or preview)",
        ))
    } else if t.iter().any(|&b| b != 0) {
        Some((Kind::Other, "Data after end of image"))
    } else {
        None
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
    !n.is_empty() && h.windows(n.len()).any(|w| w == n)
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
        fingerprint: [0; 32],
        embedded: None,
        warnings: Vec::new(),
        fields: Vec::new(),
    };
    let mut fp = Digest::new();
    let mut embedded = Digest::new();
    let mut has_embedded = false;
    let mut has_scan = false;
    for s in &segs {
        match s {
            Seg::Marker(m, whole, payload) => {
                if let Some((kind, label)) = classify(*m, payload) {
                    if kind == Kind::Exif && p.exif_tiff.is_none() {
                        p.exif_tiff = payload.get(6..).map(<[u8]>::to_vec);
                    }
                    match (*m, kind) {
                        (0xE1, Kind::Xmp)
                            if payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0") =>
                        {
                            p.fields.extend(details::xmp(&payload[29..]))
                        }
                        (0xED, Kind::Iptc) => p.fields.extend(details::iptc(payload)),
                        (0xFE, _) => p.fields.push(details::comment(payload)),
                        (0xE2, Kind::Icc) if payload.get(12) == Some(&1) => p
                            .fields
                            .extend(details::icc_header(payload.get(14..).unwrap_or_default())),
                        _ => {}
                    }
                    if kind == Kind::Embedded {
                        has_embedded = true;
                        // The index is rewritten when segments before the images move,
                        // so only its identity (not its offsets) is part of the digest.
                        embedded.add(b"MPF");
                    }
                    p.blocks.push(Block {
                        kind,
                        label,
                        size: whole.len(),
                    });
                } else {
                    if is_frame(*m) {
                        if payload.len() < 6 {
                            return Err(malformed("frame header too short"));
                        }
                        p.height = be16(payload, 1).unwrap_or(0) as u32;
                        p.width = be16(payload, 3).unwrap_or(0) as u32;
                    }
                    if *m != 0xE0 {
                        fp.add(whole);
                    }
                }
            }
            Seg::Scan(bytes) => {
                has_scan = true;
                fp.add(bytes)
            }
            Seg::Trailer(t) => {
                if let Some((kind, label)) = trailer_kind(t) {
                    if kind == Kind::Embedded {
                        has_embedded = true;
                        embedded.add(t);
                    }
                    p.blocks.push(Block {
                        kind,
                        label: label.into(),
                        size: t.len(),
                    });
                }
            }
        }
    }
    if p.width == 0 || p.height == 0 {
        return Err(malformed("no frame header"));
    }
    if !has_scan {
        return Err(malformed("no image data"));
    }
    p.fingerprint = fp.finish();
    if has_embedded {
        p.embedded = Some(embedded.finish());
    }
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
    // Positions of the MPF TIFF header and the trailer, in input and output,
    // to keep the multi-picture offsets correct when segments are removed.
    let mut pos = 2usize;
    let (mut mpf_in, mut mpf_out, mut trailer_in, mut trailer_out) = (None, None, None, None);
    for s in &segs {
        match s {
            Seg::Marker(m, whole, payload) => {
                if !exif_written && *m != 0xE0 {
                    if let Some(tiff) = minimal_exif {
                        write_exif(&mut out, tiff);
                    }
                    exif_written = true;
                }
                match classify(*m, payload) {
                    Some((kind, _)) if o.removes(kind) => {}
                    other => {
                        if matches!(other, Some((Kind::Embedded, _))) && mpf_in.is_none() {
                            mpf_in = Some(pos + 8);
                            mpf_out = Some(out.len() + 8);
                        }
                        out.extend_from_slice(whole)
                    }
                }
                pos += whole.len();
            }
            Seg::Scan(bytes) => {
                out.extend_from_slice(bytes);
                pos += bytes.len();
            }
            Seg::Trailer(t) => {
                let keep = match trailer_kind(t) {
                    Some((kind, _)) => !o.removes(kind),
                    None => true,
                };
                if keep {
                    trailer_in = Some(pos);
                    trailer_out = Some(out.len());
                    out.extend_from_slice(t);
                }
                pos += t.len();
            }
        }
    }
    if let (Some(mi), Some(mo), Some(ti), Some(to)) = (mpf_in, mpf_out, trailer_in, trailer_out) {
        let delta = (ti - mi) as i64 - (to - mo) as i64;
        if delta != 0 {
            shift_mpf_offsets(&mut out, mo, delta)
                .ok_or_else(|| malformed("multi-picture index could not be updated safely"))?;
        }
    }
    Ok(out)
}

/// Subtract `delta` from every non-zero image offset in the MP Entry table of
/// the MPF TIFF structure starting at `base`.
fn shift_mpf_offsets(out: &mut [u8], base: usize, delta: i64) -> Option<()> {
    let tiff = out.get(base..)?;
    let le = match tiff.get(0..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let r16 = |b: &[u8], i: usize| -> Option<u16> {
        let a: [u8; 2] = b.get(i..i + 2)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(a)
        } else {
            u16::from_be_bytes(a)
        })
    };
    let r32 = |b: &[u8], i: usize| -> Option<u32> {
        let a: [u8; 4] = b.get(i..i + 4)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        })
    };
    let ifd = r32(tiff, 4)? as usize;
    let n = r16(tiff, ifd)? as usize;
    if n > 64 {
        return None;
    }
    let mut table = None;
    for k in 0..n {
        let e = ifd + 2 + k * 12;
        if r16(tiff, e)? == 0xB002 {
            let count = r32(tiff, e + 4)? as usize;
            if !count.is_multiple_of(16) || count > 16 * 64 {
                return None;
            }
            let at = if count <= 4 {
                e + 8
            } else {
                r32(tiff, e + 8)? as usize
            };
            table = Some((at, count / 16));
        }
    }
    let (at, entries) = table?;
    for k in 0..entries {
        let field = base + at + k * 16 + 8;
        let old = r32(out, field)? as i64;
        if old == 0 {
            continue;
        }
        let new = u32::try_from(old - delta).ok()?;
        let bytes = if le {
            new.to_le_bytes()
        } else {
            new.to_be_bytes()
        };
        out.get_mut(field..field + 4)?.copy_from_slice(&bytes);
    }
    Some(())
}
