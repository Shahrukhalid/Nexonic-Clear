//! PNG: chunk-level parsing and rewriting. Pixel data (IDAT/fdAT) is copied as is.

use crate::{be32, malformed, Block, CleanOptions, Digest, Kind, Parsed, Result};

struct Chunk<'a> {
    ty: [u8; 4],
    data: &'a [u8],
    whole: &'a [u8],
}

/// Chunks up to and including IEND, plus any bytes after IEND.
fn chunks(d: &[u8]) -> Result<(Vec<Chunk<'_>>, &[u8])> {
    let mut out: Vec<Chunk> = Vec::new();
    let mut i = 8;
    loop {
        let len = be32(d, i).ok_or_else(|| malformed("truncated chunk header"))? as usize;
        if len > 0x7FFF_FFFF {
            return Err(malformed("chunk length out of range"));
        }
        let ty: [u8; 4] = d
            .get(i + 4..i + 8)
            .ok_or_else(|| malformed("truncated chunk type"))?
            .try_into()
            .unwrap();
        let end = i
            .checked_add(12 + len)
            .filter(|&e| e <= d.len())
            .ok_or_else(|| malformed("chunk runs past the end of the file"))?;
        if !ty.iter().all(u8::is_ascii_alphabetic) {
            return Err(malformed("invalid chunk type"));
        }
        let data = &d[i + 8..i + 8 + len];
        let mut crc = Crc::new();
        crc.add(&ty);
        crc.add(data);
        if be32(d, i + 8 + len) != Some(crc.finish()) {
            return Err(malformed("chunk checksum mismatch"));
        }
        match (&ty, out.is_empty()) {
            (b"IHDR", true) if len == 13 => {}
            (b"IHDR", _) => return Err(malformed("invalid or repeated IHDR")),
            (_, true) => return Err(malformed("IHDR must be the first chunk")),
            _ => {}
        }
        out.push(Chunk {
            ty,
            data,
            whole: &d[i..end],
        });
        i = end;
        if &ty == b"IEND" {
            if !out.iter().any(|c| &c.ty == b"IDAT") {
                return Err(malformed("no image data"));
            }
            return Ok((out, &d[i..]));
        }
    }
}

fn keyword(data: &[u8]) -> &[u8] {
    let n = data
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(data.len().min(79));
    &data[..n]
}

fn classify(c: &Chunk) -> Option<(Kind, String)> {
    let t = &c.ty;
    match t {
        b"eXIf" => Some((Kind::Exif, "eXIf chunk".into())),
        b"iCCP" => Some((Kind::Icc, "iCCP colour profile".into())),
        b"caBX" => Some((Kind::C2pa, "caBX C2PA manifest".into())),
        b"tIME" => Some((Kind::Other, "tIME modification time".into())),
        b"tEXt" | b"zTXt" | b"iTXt" => {
            let k = keyword(c.data);
            let ks = String::from_utf8_lossy(k).to_string();
            let lk = ks.to_ascii_lowercase();
            let kind = if lk == "xml:com.adobe.xmp" {
                Kind::Xmp
            } else if lk.starts_with("raw profile type exif")
                || lk.starts_with("raw profile type app1")
            {
                Kind::Exif
            } else if lk.starts_with("raw profile type iptc")
                || lk.starts_with("raw profile type 8bim")
            {
                Kind::Iptc
            } else if lk.starts_with("raw profile type icc")
                || lk.starts_with("raw profile type icm")
            {
                Kind::Icc
            } else if lk.starts_with("raw profile type xmp") {
                Kind::Xmp
            } else {
                Kind::Comment
            };
            Some((kind, format!("{} \"{}\"", String::from_utf8_lossy(t), ks)))
        }
        // Image structure and colour interpretation: always kept.
        b"IHDR" | b"PLTE" | b"IDAT" | b"IEND" | b"tRNS" | b"gAMA" | b"cHRM" | b"sRGB" | b"sBIT"
        | b"pHYs" | b"bKGD" | b"hIST" | b"sPLT" | b"acTL" | b"fcTL" | b"fdAT" | b"cICP"
        | b"mDCV" | b"cLLI" => None,
        _ if t[0].is_ascii_uppercase() => None, // unknown critical chunk: must be kept
        _ => Some((Kind::Other, format!("{} chunk", String::from_utf8_lossy(t)))),
    }
}

fn pixel_relevant(t: &[u8; 4]) -> bool {
    matches!(
        t,
        b"IHDR"
            | b"PLTE"
            | b"IDAT"
            | b"tRNS"
            | b"gAMA"
            | b"cHRM"
            | b"sRGB"
            | b"cICP"
            | b"acTL"
            | b"fcTL"
            | b"fdAT"
            | b"IEND"
    )
}

pub fn parse(d: &[u8]) -> Result<Parsed> {
    let (cs, trailer) = chunks(d)?;
    let ihdr = cs
        .first()
        .filter(|c| &c.ty == b"IHDR" && c.data.len() >= 13)
        .ok_or_else(|| malformed("missing IHDR"))?;
    let color_type = ihdr.data[9];
    let mut p = Parsed {
        width: be32(ihdr.data, 0).unwrap(),
        height: be32(ihdr.data, 4).unwrap(),
        has_alpha: color_type == 4 || color_type == 6 || cs.iter().any(|c| &c.ty == b"tRNS"),
        animated: cs.iter().any(|c| &c.ty == b"acTL"),
        blocks: Vec::new(),
        exif_tiff: None,
        fingerprint: [0; 32],
        embedded: None,
        warnings: Vec::new(),
    };
    if p.width == 0 || p.height == 0 {
        return Err(malformed("invalid image dimensions"));
    }
    let mut fp = Digest::new();
    for c in &cs {
        if let Some((kind, label)) = classify(c) {
            if &c.ty == b"eXIf" && p.exif_tiff.is_none() {
                let data = c.data.strip_prefix(b"Exif\0\0").unwrap_or(c.data);
                p.exif_tiff = Some(data.to_vec());
            }
            p.blocks.push(Block {
                kind,
                label,
                size: c.whole.len(),
            });
        } else if pixel_relevant(&c.ty) {
            fp.add(c.whole);
        }
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
    let mut out = Vec::with_capacity(d.len());
    out.extend_from_slice(&d[..8]);
    let mut pending = minimal_exif;
    for c in &cs {
        if let Some(tiff) = pending {
            if matches!(&c.ty, b"IDAT" | b"acTL" | b"fcTL") {
                write_chunk(&mut out, b"eXIf", tiff);
                pending = None;
            }
        }
        match classify(c) {
            Some((kind, _)) if o.removes(kind) => {}
            _ => out.extend_from_slice(c.whole),
        }
    }
    if !o.other {
        out.extend_from_slice(trailer);
    }
    Ok(out)
}

fn write_chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    let mut crc = Crc::new();
    crc.add(ty);
    crc.add(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

struct Crc(u32);
impl Crc {
    fn new() -> Self {
        Crc(0xFFFF_FFFF)
    }
    fn add(&mut self, bytes: &[u8]) {
        for &b in bytes {
            let mut c = (self.0 ^ b as u32) & 0xFF;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            self.0 = (self.0 >> 8) ^ c;
        }
    }
    fn finish(&self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}
