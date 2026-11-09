//! WebP: RIFF chunk-level parsing and rewriting. Bitstream chunks are copied as is.

use crate::{le24, le32, malformed, Block, CleanOptions, Digest, Kind, Parsed, Result};

const F_ICC: u8 = 0x20;
const F_ALPHA: u8 = 0x10;
const F_EXIF: u8 = 0x08;
const F_XMP: u8 = 0x04;
const F_ANIM: u8 = 0x02;

struct Chunk<'a> {
    id: [u8; 4],
    data: &'a [u8],
    whole: &'a [u8],
}

/// RIFF chunks, plus any bytes after the RIFF container.
fn chunks(d: &[u8]) -> Result<(Vec<Chunk<'_>>, &[u8])> {
    let riff = le32(d, 4).ok_or_else(|| malformed("truncated header"))? as usize;
    let end = riff
        .checked_add(8)
        .filter(|&e| e <= d.len() && riff >= 4)
        .ok_or_else(|| malformed("file is shorter than its header says"))?;
    let mut out: Vec<Chunk> = Vec::new();
    let mut i = 12;
    while i < end {
        if i + 8 > end {
            return Err(malformed("truncated chunk header"));
        }
        let id: [u8; 4] = d[i..i + 4].try_into().unwrap();
        let len = le32(d, i + 4).unwrap() as usize;
        let data_end = i
            .checked_add(8 + len)
            .filter(|&e| e <= end)
            .ok_or_else(|| malformed("chunk runs past the end of the file"))?;
        let padded = data_end + (len & 1);
        if padded > end {
            return Err(malformed("missing chunk padding"));
        }
        if &id == b"VP8X" && !out.is_empty() {
            return Err(malformed("VP8X must be the first chunk"));
        }
        out.push(Chunk {
            id,
            data: &d[i + 8..data_end],
            whole: &d[i..padded],
        });
        i = padded;
    }
    let has = |id: &[u8; 4]| out.iter().any(|c| &c.id == id);
    if !(has(b"VP8 ") || has(b"VP8L") || (has(b"ANIM") && has(b"ANMF"))) {
        return Err(malformed("no image data"));
    }
    if (has(b"ALPH") || has(b"ANIM")) && !has(b"VP8X") {
        return Err(malformed("extended features without VP8X"));
    }
    Ok((out, &d[end..]))
}

fn classify(id: &[u8; 4]) -> Option<(Kind, &'static str)> {
    match id {
        b"EXIF" => Some((Kind::Exif, "EXIF chunk")),
        b"XMP " => Some((Kind::Xmp, "XMP chunk")),
        b"ICCP" => Some((Kind::Icc, "ICCP colour profile")),
        b"C2PA" => Some((Kind::C2pa, "C2PA manifest")),
        b"VP8 " | b"VP8L" | b"VP8X" | b"ALPH" | b"ANIM" | b"ANMF" => None,
        _ => Some((Kind::Other, "Unknown chunk")),
    }
}

pub fn parse(d: &[u8]) -> Result<Parsed> {
    let (cs, trailer) = chunks(d)?;
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
    };
    let mut fp = Digest::new();
    for c in &cs {
        match &c.id {
            b"VP8X" if c.data.len() >= 10 => {
                p.has_alpha = c.data[0] & F_ALPHA != 0;
                p.animated = c.data[0] & F_ANIM != 0;
                p.width = le24(c.data, 4).unwrap() + 1;
                p.height = le24(c.data, 7).unwrap() + 1;
            }
            b"VP8 " if c.data.len() >= 10 && p.width == 0 => {
                if c.data[3..6] != [0x9D, 0x01, 0x2A] {
                    return Err(malformed("bad VP8 start code"));
                }
                p.width = (u16::from_le_bytes([c.data[6], c.data[7]]) & 0x3FFF) as u32;
                p.height = (u16::from_le_bytes([c.data[8], c.data[9]]) & 0x3FFF) as u32;
            }
            b"VP8L" if c.data.len() >= 5 && p.width == 0 => {
                if c.data[0] != 0x2F {
                    return Err(malformed("bad VP8L signature"));
                }
                let bits = le32(c.data, 1).unwrap();
                p.width = (bits & 0x3FFF) + 1;
                p.height = ((bits >> 14) & 0x3FFF) + 1;
                p.has_alpha = (bits >> 28) & 1 == 1;
            }
            _ => {}
        }
        if let Some((kind, label)) = classify(&c.id) {
            if kind == Kind::Exif && p.exif_tiff.is_none() {
                p.exif_tiff = Some(c.data.strip_prefix(b"Exif\0\0").unwrap_or(c.data).to_vec());
            }
            let label = if kind == Kind::Other {
                format!("{} chunk", String::from_utf8_lossy(&c.id).trim())
            } else {
                label.into()
            };
            p.blocks.push(Block {
                kind,
                label,
                size: c.whole.len(),
            });
        } else if &c.id != b"VP8X" {
            fp.add(c.whole);
        }
    }
    if p.width == 0 || p.height == 0 {
        return Err(malformed("no image data"));
    }
    if trailer.iter().any(|&b| b != 0) {
        p.blocks.push(Block {
            kind: Kind::Other,
            label: "Data after end of image".into(),
            size: trailer.len(),
        });
    }
    p.fingerprint = fp.finish();
    Ok(p)
}

pub fn clean(d: &[u8], o: &CleanOptions, minimal_exif: Option<&[u8]>) -> Result<Vec<u8>> {
    let (cs, trailer) = chunks(d)?;
    let has_vp8x = cs.iter().any(|c| &c.id == b"VP8X");
    let minimal_exif = minimal_exif.filter(|_| has_vp8x);
    let mut out = Vec::with_capacity(d.len());
    out.extend_from_slice(b"RIFF\0\0\0\0WEBP");
    let mut kept = std::collections::HashSet::new();
    for c in &cs {
        match classify(&c.id) {
            Some((kind, _)) if o.removes(kind) => {
                if kind == Kind::Exif {
                    if let Some(tiff) = minimal_exif {
                        write_chunk(&mut out, b"EXIF", tiff);
                        kept.insert(Kind::Exif);
                    }
                }
            }
            Some((kind, _)) => {
                kept.insert(kind);
                out.extend_from_slice(c.whole);
            }
            None => out.extend_from_slice(c.whole),
        }
    }
    // Make the VP8X feature flags match the chunks that remain.
    if let Some(pos) = find_vp8x(&out) {
        let mut flags = out[pos] & !(F_ICC | F_EXIF | F_XMP);
        if kept.contains(&Kind::Icc) {
            flags |= F_ICC;
        }
        if kept.contains(&Kind::Exif) {
            flags |= F_EXIF;
        }
        if kept.contains(&Kind::Xmp) {
            flags |= F_XMP;
        }
        out[pos] = flags;
    }
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    if !o.other {
        out.extend_from_slice(trailer);
    }
    Ok(out)
}

fn find_vp8x(out: &[u8]) -> Option<usize> {
    (out.len() >= 12 + 18 && &out[12..16] == b"VP8X").then_some(12 + 8)
}

fn write_chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() & 1 == 1 {
        out.push(0);
    }
}
