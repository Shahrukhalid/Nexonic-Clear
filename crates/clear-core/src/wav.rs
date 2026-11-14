//! WAV (RIFF/WAVE and RF64): chunk-level parsing and rewriting. Audio chunks
//! are copied byte for byte and fingerprinted.

use crate::stream::{digest_range, Plan, Src};
use crate::{details, malformed, Block, CleanOptions, Digest, Field, Kind, Parsed, Result};

struct Chunk {
    id: [u8; 4],
    /// Start of the chunk header.
    at: u64,
    /// Payload length.
    len: u64,
    /// Header + payload + padding.
    whole: u64,
}

struct Riff {
    rf64: bool,
    chunks: Vec<Chunk>,
    end: u64,
}

fn le32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn le64(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}

fn read(src: &dyn Src) -> Result<Riff> {
    let h = src.bytes(0, 12)?;
    let rf64 = &h[0..4] == b"RF64" || &h[0..4] == b"BW64";
    let mut end = 8 + le32(&h, 4) as u64;
    let mut data_size: Option<u64> = None;
    if rf64 {
        let ds = src.bytes(12, 8)?;
        if &ds[0..4] != b"ds64" || le32(&ds, 4) < 24 {
            return Err(malformed("RF64 file without a ds64 chunk"));
        }
        let body = src.bytes(20, 24)?;
        end = le64(&body, 0).saturating_add(8);
        data_size = Some(le64(&body, 8));
    }
    if end > src.len() || end < 12 {
        return Err(malformed("file is shorter than its header says"));
    }
    let mut chunks = Vec::new();
    let mut i = 12u64;
    while i < end {
        if i + 8 > end {
            return Err(malformed("truncated chunk header"));
        }
        let ch = src.bytes(i, 8)?;
        let id: [u8; 4] = ch[0..4].try_into().unwrap();
        let mut len = le32(&ch, 4) as u64;
        if rf64 && &id == b"data" && len == 0xFFFF_FFFF {
            len = data_size.unwrap_or(0);
        }
        let data_end = (i + 8)
            .checked_add(len)
            .filter(|&e| e <= end)
            .ok_or_else(|| malformed("chunk runs past the end of the file"))?;
        let padded = (data_end + (len & 1)).min(end);
        chunks.push(Chunk {
            id,
            at: i,
            len,
            whole: padded - i,
        });
        i = padded;
    }
    if !chunks.iter().any(|c| &c.id == b"fmt ") || !chunks.iter().any(|c| &c.id == b"data") {
        return Err(malformed("no audio data"));
    }
    Ok(Riff { rf64, chunks, end })
}

fn classify(id: &[u8; 4], list_type: Option<&[u8]>) -> Option<(Kind, String)> {
    let name = || String::from_utf8_lossy(id).trim().to_string();
    match id {
        b"fmt " | b"data" | b"fact" | b"cue " | b"smpl" | b"inst" | b"acid" | b"plst" | b"ds64" => {
            None
        }
        b"LIST" => match list_type {
            Some(b"INFO") => Some((Kind::Comment, "LIST INFO tags".into())),
            Some(b"adtl") => Some((Kind::Comment, "LIST adtl labels".into())),
            Some(t) => Some((
                Kind::Other,
                format!("LIST {} chunk", String::from_utf8_lossy(t).trim()),
            )),
            None => Some((Kind::Other, "LIST chunk".into())),
        },
        b"id3 " | b"ID3 " => Some((Kind::Comment, "ID3 tag chunk".into())),
        b"bext" => Some((Kind::Comment, "Broadcast WAV (bext)".into())),
        b"iXML" => Some((Kind::Comment, "iXML chunk".into())),
        b"axml" => Some((Kind::Comment, "axml chunk".into())),
        b"cart" => Some((Kind::Comment, "cart chunk".into())),
        b"_PMX" => Some((Kind::Xmp, "XMP chunk (_PMX)".into())),
        b"C2PA" => Some((Kind::C2pa, "C2PA manifest".into())),
        _ => Some((Kind::Other, format!("{} chunk", name()))),
    }
}

fn info_name(id: &[u8]) -> Option<&'static str> {
    Some(match id {
        b"INAM" => "Title",
        b"IART" => "Artist",
        b"IPRD" => "Album",
        b"ICMT" => "Comment",
        b"ICRD" => "Date",
        b"IGNR" => "Genre",
        b"ISFT" => "Software",
        b"ICOP" => "Copyright",
        b"IENG" => "Engineer",
        b"ITRK" | b"IPRT" => "Track",
        b"ISRC" => "Source",
        b"ISBJ" => "Subject",
        b"IKEY" => "Keywords",
        b"ITCH" => "Technician",
        _ => return None,
    })
}

fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

fn field(group: &str, name: &str, value: String) -> Field {
    Field {
        group: group.into(),
        name: name.into(),
        value,
        tag: None,
    }
}

pub(crate) fn parse(src: &dyn Src) -> Result<Parsed> {
    let r = read(src)?;
    let mut p = Parsed::empty();
    let mut fp = Digest::new();
    let mut byte_rate = 0u32;
    for c in &r.chunks {
        let list_type = if &c.id == b"LIST" && c.len >= 4 {
            Some(src.bytes(c.at + 8, 4)?)
        } else {
            None
        };
        match classify(&c.id, list_type.as_deref()) {
            None => {
                if &c.id == b"fmt " && c.len >= 16 {
                    let f = src.bytes(c.at + 8, 16)?;
                    let channels = u16::from_le_bytes([f[2], f[3]]);
                    let rate = le32(&f, 4);
                    byte_rate = le32(&f, 8);
                    let bits = u16::from_le_bytes([f[14], f[15]]);
                    p.fields
                        .push(field("Audio", "Channels", channels.to_string()));
                    p.fields
                        .push(field("Audio", "Sample rate", format!("{rate} Hz")));
                    p.fields
                        .push(field("Audio", "Bits per sample", bits.to_string()));
                }
                if &c.id == b"data" && byte_rate > 0 {
                    let secs = c.len as f64 / byte_rate as f64;
                    p.fields
                        .push(field("Audio", "Duration", format!("{secs:.2} s")));
                }
                // ds64 holds sizes that change when metadata is removed.
                if &c.id != b"ds64" {
                    let id_len = if &c.id == b"data" { c.len } else { c.whole - 8 };
                    fp.add(&c.id);
                    digest_range(&mut fp, src, c.at + 8, id_len)?;
                }
            }
            Some((kind, label)) => {
                if c.len <= 1 << 20 {
                    let body = src.bytes(c.at + 8, c.len)?;
                    match (&c.id, list_type.as_deref()) {
                        (b"LIST", Some(b"INFO")) => {
                            let mut i = 4;
                            while i + 8 <= body.len() {
                                let id = &body[i..i + 4];
                                let n = le32(&body, i + 4) as usize;
                                let Some(v) = body.get(i + 8..i + 8 + n) else {
                                    break;
                                };
                                let name = info_name(id)
                                    .map(str::to_string)
                                    .unwrap_or_else(|| String::from_utf8_lossy(id).into());
                                p.fields.push(field("INFO", &name, text(v)));
                                i += 8 + n + (n & 1);
                            }
                        }
                        (b"bext", _) if body.len() >= 346 => {
                            for (name, a, b) in [
                                ("Description", 0, 256),
                                ("Originator", 256, 288),
                                ("Originator reference", 288, 320),
                                ("Origination date", 320, 330),
                                ("Origination time", 330, 338),
                            ] {
                                let v = text(&body[a..b]);
                                if !v.is_empty() {
                                    p.fields.push(field("BWF", name, v));
                                }
                            }
                        }
                        (b"_PMX", _) => p.fields.extend(details::xmp(&body)),
                        _ => {}
                    }
                }
                p.blocks.push(Block {
                    kind,
                    label,
                    size: c.whole as usize,
                });
            }
        }
    }
    if r.end < src.len() {
        p.blocks.push(Block {
            kind: Kind::Other,
            label: "Data after end of audio".into(),
            size: (src.len() - r.end) as usize,
        });
    }
    p.fingerprint = fp.finish();
    Ok(p)
}

pub(crate) fn clean(src: &dyn Src, o: &CleanOptions) -> Result<Plan> {
    let r = read(src)?;
    let mut plan = Plan::default();
    let mut header = src.bytes(0, 12)?;
    let mut ds64: Option<Vec<u8>> = None;
    let mut body = Plan::default();
    for c in &r.chunks {
        let list_type = if &c.id == b"LIST" && c.len >= 4 {
            Some(src.bytes(c.at + 8, 4)?)
        } else {
            None
        };
        match classify(&c.id, list_type.as_deref()) {
            Some((kind, _)) if o.removes(kind) => {}
            _ if &c.id == b"ds64" && r.rf64 && ds64.is_none() => {
                ds64 = Some(src.bytes(c.at, c.whole)?);
            }
            _ => body.copy(c.at, c.whole),
        }
    }
    let total = 12 + ds64.as_ref().map_or(0, |d| d.len() as u64) + body.len();
    if let Some(d) = ds64.as_mut() {
        d[8..16].copy_from_slice(&(total - 8).to_le_bytes());
    } else {
        let size = u32::try_from(total - 8).map_err(|_| malformed("file too large for WAV"))?;
        header[4..8].copy_from_slice(&size.to_le_bytes());
    }
    plan.bytes(header);
    if let Some(d) = ds64 {
        plan.bytes(d);
    }
    plan.append(body);
    if !o.other && r.end < src.len() {
        plan.copy(r.end, src.len() - r.end);
    }
    Ok(plan)
}
