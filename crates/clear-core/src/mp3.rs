//! MP3: ID3v2 tags are rewritten frame by frame; trailing ID3v1, Lyrics3 and
//! APEv2 tags are kept or dropped whole. The MPEG audio between the tags is
//! copied byte for byte and fingerprinted.

use crate::stream::{digest_range, Plan, Src};
use crate::{details, malformed, Block, CleanOptions, Digest, Field, Kind, Parsed, Result};

struct Frame {
    kind: Kind,
    label: String,
    /// Offset of the frame inside the tag body (after the 10-byte header).
    start: usize,
    end: usize,
}

struct Tag {
    at: u64,
    whole: u64,
    raw: Vec<u8>,
    version: u8,
    unsync: bool,
    frames: Vec<Frame>,
    /// Bytes after the last frame that were not padding.
    rest: Option<(usize, usize)>,
}

struct Trailer {
    kind: Kind,
    label: &'static str,
    at: u64,
    len: u64,
}

struct Layout {
    tags: Vec<Tag>,
    audio: (u64, u64),
    trailers: Vec<Trailer>,
}

fn syncsafe(b: &[u8]) -> u32 {
    b.iter()
        .take(4)
        .fold(0, |a, &x| (a << 7) | (x & 0x7F) as u32)
}

fn deunsync(b: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        o.push(b[i]);
        if b[i] == 0xFF && b.get(i + 1) == Some(&0) {
            i += 1;
        }
        i += 1;
    }
    o
}

fn cstr(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

fn classify(id: &str, body: &[u8]) -> (Kind, String) {
    let lab = |s: &str| format!("ID3 {id} {s}").trim().to_string();
    match id {
        "GEOB" | "GEO" => {
            let mime = body.get(1..).map(cstr).unwrap_or_default();
            if mime.eq_ignore_ascii_case(b"application/c2pa")
                || mime.eq_ignore_ascii_case(b"application/x-c2pa-manifest-store")
            {
                (Kind::C2pa, "C2PA manifest (ID3 GEOB)".into())
            } else {
                (Kind::Other, lab("object"))
            }
        }
        "PRIV" if cstr(body) == b"XMP" => (Kind::Xmp, "XMP (ID3 PRIV)".into()),
        "APIC" | "PIC" => (Kind::Embedded, lab("cover art")),
        "COMM" | "COM" | "USLT" | "ULT" => (Kind::Comment, lab("")),
        _ if id.starts_with('T') || id.starts_with('W') => (Kind::Comment, lab("")),
        _ => (Kind::Other, lab("")),
    }
}

fn read_tag(src: &dyn Src, at: u64) -> Result<Option<Tag>> {
    if at + 10 > src.len() {
        return Ok(None);
    }
    let h = src.bytes(at, 10)?;
    if &h[0..3] != b"ID3" || h[3] == 0xFF || h[6..10].iter().any(|&b| b & 0x80 != 0) {
        return Ok(None);
    }
    let version = h[3];
    if !(2..=4).contains(&version) {
        return Err(malformed("unknown ID3 version"));
    }
    let size = syncsafe(&h[6..10]) as u64;
    let footer = if version == 4 && h[5] & 0x10 != 0 {
        10
    } else {
        0
    };
    let whole = 10 + size + footer;
    let raw = src.bytes(at, whole)?;
    let unsync = h[5] & 0x80 != 0;
    let body_raw = &raw[10..10 + size as usize];
    let body: Vec<u8> = if unsync && version < 4 {
        deunsync(body_raw)
    } else {
        body_raw.to_vec()
    };
    let mut i = 0usize;
    if h[5] & 0x40 != 0 {
        let ext = if version == 4 {
            syncsafe(body.get(0..4).unwrap_or(&[0; 4])) as usize
        } else {
            4 + crate::be32(&body, 0).unwrap_or(0) as usize
        };
        i = ext.min(body.len());
    }
    let (id_len, hdr) = if version == 2 { (3, 6) } else { (4, 10) };
    let mut frames = Vec::new();
    let mut rest = None;
    while i + hdr <= body.len() {
        if body[i] == 0 {
            if body[i..].iter().any(|&b| b != 0) {
                rest = Some((i, body.len()));
            }
            break;
        }
        let id = String::from_utf8_lossy(&body[i..i + id_len]).to_string();
        let n = match version {
            2 => (body[i + 3] as usize) << 16 | (body[i + 4] as usize) << 8 | body[i + 5] as usize,
            3 => crate::be32(&body, i + 4).unwrap() as usize,
            _ => syncsafe(&body[i + 4..i + 8]) as usize,
        };
        let end = i + hdr + n;
        if !id
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            || end > body.len()
        {
            rest = Some((i, body.len()));
            break;
        }
        let (kind, label) = classify(&id, &body[i + hdr..end]);
        frames.push(Frame {
            kind,
            label,
            start: i,
            end,
        });
        i = end;
    }
    // Keep the de-unsynchronised body for reading; offsets refer to it.
    let mut raw2 = raw[..10].to_vec();
    raw2.extend_from_slice(&body);
    Ok(Some(Tag {
        at,
        whole,
        raw: raw2,
        version,
        unsync,
        frames,
        rest,
    }))
}

/// MPEG audio frame length at `b[0..4]`, if it is a valid header.
fn frame_len(b: &[u8]) -> Option<(usize, u32, u32)> {
    if b.len() < 4 || b[0] != 0xFF || b[1] & 0xE0 != 0xE0 {
        return None;
    }
    let ver = (b[1] >> 3) & 3; // 0: 2.5, 2: 2, 3: 1
    let layer = (b[1] >> 1) & 3; // 1: III, 2: II, 3: I
    let bi = (b[2] >> 4) as usize;
    let si = ((b[2] >> 2) & 3) as usize;
    if ver == 1 || layer == 0 || bi == 0 || bi == 15 || si == 3 {
        return None;
    }
    const V1: [[u32; 15]; 3] = [
        [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ],
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
        ],
        [
            0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
        ],
    ];
    const V2: [[u32; 15]; 2] = [
        [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
        [
            0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
        ],
    ];
    let kbps = if ver == 3 {
        V1[layer as usize - 1][bi]
    } else if layer == 3 {
        V2[1][bi]
    } else {
        V2[0][bi]
    };
    let base = [44100, 48000, 32000][si];
    let rate = match ver {
        3 => base,
        2 => base / 2,
        _ => base / 4,
    };
    let pad = ((b[2] >> 1) & 1) as u32;
    let len = if layer == 3 {
        (12 * kbps * 1000 / rate + pad) * 4
    } else if layer == 1 && ver != 3 {
        72 * kbps * 1000 / rate + pad
    } else {
        144 * kbps * 1000 / rate + pad
    };
    Some((len as usize, kbps, rate))
}

fn trailers(src: &dyn Src, mut end: u64, floor: u64) -> Result<(Vec<Trailer>, u64)> {
    let mut out = Vec::new();
    for _ in 0..4 {
        if end >= floor + 128 && src.bytes(end - 128, 3)? == b"TAG" {
            out.push(Trailer {
                kind: Kind::Comment,
                label: "ID3v1 tag",
                at: end - 128,
                len: 128,
            });
            end -= 128;
            continue;
        }
        if end >= floor + 32 && src.bytes(end - 32, 8)? == b"APETAGEX" {
            let f = src.bytes(end - 32, 32)?;
            let size = crate::le32(&f, 12).unwrap() as u64;
            let flags = crate::le32(&f, 20).unwrap();
            let len = size + if flags & 0x8000_0000 != 0 { 32 } else { 0 };
            if len >= 32 && end >= floor + len {
                out.push(Trailer {
                    kind: Kind::Other,
                    label: "APEv2 tag",
                    at: end - len,
                    len,
                });
                end -= len;
                continue;
            }
        }
        if end >= floor + 15 && src.bytes(end - 9, 9)? == b"LYRICS200" {
            let s = src.bytes(end - 15, 6)?;
            if let Some(n) = std::str::from_utf8(&s)
                .ok()
                .and_then(|t| t.parse::<u64>().ok())
            {
                let len = n + 15;
                if end >= floor + len && src.bytes(end - len, 11)? == b"LYRICSBEGIN" {
                    out.push(Trailer {
                        kind: Kind::Comment,
                        label: "Lyrics3 tag",
                        at: end - len,
                        len,
                    });
                    end -= len;
                    continue;
                }
            }
        }
        break;
    }
    out.reverse();
    Ok((out, end))
}

fn layout(src: &dyn Src) -> Result<(Layout, Vec<Field>)> {
    let mut tags = Vec::new();
    let mut at = 0u64;
    while let Some(t) = read_tag(src, at)? {
        at += t.whole;
        tags.push(t);
    }
    let (trailers, end) = trailers(src, src.len(), at)?;
    if end <= at {
        return Err(malformed("no audio data"));
    }
    // Find frame sync near the start and check three frames in a row.
    let probe = src.bytes(at, (end - at).min(64 * 1024))?;
    let mut fields = Vec::new();
    let mut ok = false;
    for s in 0..probe.len().saturating_sub(4) {
        let Some((len, kbps, rate)) = frame_len(&probe[s..]) else {
            continue;
        };
        let mut pos = at + s as u64 + len as u64;
        let mut good = 1;
        while good < 3 && pos + 4 <= end {
            let h = src.bytes(pos, 4)?;
            let Some((l, _, _)) = frame_len(&h) else {
                break;
            };
            pos += l as u64;
            good += 1;
        }
        if good == 3 || pos == end {
            ok = true;
            let mode = if probe[s + 3] >> 6 == 3 {
                "Mono"
            } else {
                "Stereo"
            };
            fields.push(f("Audio", "Sample rate", format!("{rate} Hz")));
            fields.push(f("Audio", "Bit rate", format!("{kbps} kbit/s")));
            fields.push(f("Audio", "Channels", mode.into()));
            break;
        }
    }
    if !ok {
        return Err(malformed("no MPEG audio frames"));
    }
    Ok((
        Layout {
            tags,
            audio: (at, end - at),
            trailers,
        },
        fields,
    ))
}

fn f(group: &str, name: &str, value: String) -> Field {
    Field {
        group: group.into(),
        name: name.into(),
        value,
        tag: None,
    }
}

fn decode(enc: u8, b: &[u8]) -> String {
    let s = match enc {
        1 | 2 => {
            let (be, b) = match b {
                [0xFE, 0xFF, r @ ..] => (true, r),
                [0xFF, 0xFE, r @ ..] => (false, r),
                _ => (enc == 2, b),
            };
            let u: Vec<u16> = b
                .chunks_exact(2)
                .map(|c| {
                    if be {
                        u16::from_be_bytes([c[0], c[1]])
                    } else {
                        u16::from_le_bytes([c[0], c[1]])
                    }
                })
                .collect();
            String::from_utf16_lossy(&u)
        }
        3 => String::from_utf8_lossy(b).into(),
        _ => b.iter().map(|&c| c as char).collect(),
    };
    s.trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .replace('\0', " / ")
}

fn frame_name(id: &str) -> &str {
    match id {
        "TIT2" | "TT2" => "Title",
        "TPE1" | "TP1" => "Artist",
        "TPE2" | "TP2" => "Album artist",
        "TALB" | "TAL" => "Album",
        "TYER" | "TYE" | "TDRC" => "Year",
        "TCON" | "TCO" => "Genre",
        "TRCK" | "TRK" => "Track",
        "TENC" | "TEN" => "Encoded by",
        "TSSE" | "TSS" => "Encoder",
        "TCOP" | "TCR" => "Copyright",
        "TCOM" | "TCM" => "Composer",
        "COMM" | "COM" => "Comment",
        "USLT" | "ULT" => "Lyrics",
        _ => id,
    }
}

fn tag_fields(t: &Tag, out: &mut Vec<Field>) {
    let hdr = if t.version == 2 { 6 } else { 10 };
    let id_len = if t.version == 2 { 3 } else { 4 };
    for fr in &t.frames {
        let b = &t.raw[10 + fr.start..10 + fr.end];
        let id = String::from_utf8_lossy(&b[..id_len]).to_string();
        let body = &b[hdr..];
        if body.is_empty() {
            continue;
        }
        if id.starts_with('T') && id != "TXXX" && id != "TXX" {
            out.push(f("ID3", frame_name(&id), decode(body[0], &body[1..])));
        } else if (id == "COMM" || id == "COM" || id == "USLT" || id == "ULT") && body.len() > 4 {
            out.push(f("ID3", frame_name(&id), decode(body[0], &body[4..])));
        } else if id == "TXXX" || id == "TXX" {
            out.push(f("ID3", "User text", decode(body[0], &body[1..])));
        } else if id == "PRIV" && cstr(body) == b"XMP" {
            out.extend(details::xmp(&body[4..]));
        } else if id.starts_with('W') {
            out.push(f("ID3", &id, decode(0, body)));
        }
    }
}

pub(crate) fn parse(src: &dyn Src) -> Result<Parsed> {
    let (l, fields) = layout(src)?;
    let mut p = Parsed::empty();
    p.fields = fields;
    for t in &l.tags {
        tag_fields(t, &mut p.fields);
        for fr in &t.frames {
            p.blocks.push(Block {
                kind: fr.kind,
                label: fr.label.clone(),
                size: fr.end - fr.start,
            });
        }
        if let Some((a, b)) = t.rest {
            p.blocks.push(Block {
                kind: Kind::Other,
                label: "Unreadable ID3 data".into(),
                size: b - a,
            });
        }
        if t.unsync && t.frames.len() > 1 {
            p.warnings.push(
                "An ID3 tag uses unsynchronisation, so it can only be kept or removed as a whole."
                    .into(),
            );
        }
    }
    for tr in &l.trailers {
        if tr.label == "ID3v1 tag" {
            let b = src.bytes(tr.at, 128)?;
            for (name, a, z) in [
                ("Title", 3, 33),
                ("Artist", 33, 63),
                ("Album", 63, 93),
                ("Year", 93, 97),
                ("Comment", 97, 127),
            ] {
                let v = decode(0, cstr(&b[a..z]));
                if !v.is_empty() {
                    p.fields.push(f("ID3v1", name, v));
                }
            }
        }
        p.blocks.push(Block {
            kind: tr.kind,
            label: tr.label.into(),
            size: tr.len as usize,
        });
    }
    let mut fp = Digest::new();
    digest_range(&mut fp, src, l.audio.0, l.audio.1)?;
    p.fingerprint = fp.finish();
    // Cover art counts as embedded content: track it so keeping it is verified.
    let mut emb = Digest::new();
    let mut any = false;
    for t in &l.tags {
        for fr in t.frames.iter().filter(|fr| fr.kind == Kind::Embedded) {
            let hdr = if t.version == 2 { 6 } else { 10 };
            emb.add(&t.raw[10 + fr.start + hdr..10 + fr.end]);
            any = true;
        }
    }
    p.embedded = any.then(|| emb.finish());
    Ok(p)
}

pub(crate) fn clean(src: &dyn Src, o: &CleanOptions) -> Result<Plan> {
    let (l, _) = layout(src)?;
    let mut plan = Plan::default();
    for t in &l.tags {
        let rest_removed = t.rest.is_some() && o.other;
        let removed: Vec<bool> = t.frames.iter().map(|fr| o.removes(fr.kind)).collect();
        if removed.iter().all(|&r| r) && (t.rest.is_none() || rest_removed) {
            continue;
        }
        if !removed.iter().any(|&r| r) && !rest_removed {
            plan.copy(t.at, t.whole);
            continue;
        }
        if t.unsync {
            // Frame boundaries in the raw bytes are not reliable: keep it whole.
            plan.copy(t.at, t.whole);
            continue;
        }
        let mut body = Vec::new();
        for (fr, r) in t.frames.iter().zip(&removed) {
            if !r {
                body.extend_from_slice(&t.raw[10 + fr.start..10 + fr.end]);
            }
        }
        if let (Some((a, b)), false) = (t.rest, rest_removed) {
            body.extend_from_slice(&t.raw[10 + a..10 + b]);
        }
        if body.is_empty() {
            continue;
        }
        let n = body.len() as u32;
        let mut h = t.raw[..10].to_vec();
        h[5] &= !(0x40 | 0x10 | 0x80);
        h[6..10].copy_from_slice(&[
            (n >> 21) as u8 & 0x7F,
            (n >> 14) as u8 & 0x7F,
            (n >> 7) as u8 & 0x7F,
            n as u8 & 0x7F,
        ]);
        h.extend_from_slice(&body);
        plan.bytes(h);
    }
    if plan.0.is_empty() && frame_len(&src.bytes(l.audio.0, 4.min(l.audio.1))?).is_none() {
        // Without a tag in front, the file must start with an MPEG frame to
        // be recognised as MP3 again.
        return Err(malformed("the audio does not start with an MPEG frame"));
    }
    plan.copy(l.audio.0, l.audio.1);
    for tr in &l.trailers {
        if !o.removes(tr.kind) {
            plan.copy(tr.at, tr.len);
        }
    }
    Ok(plan)
}
