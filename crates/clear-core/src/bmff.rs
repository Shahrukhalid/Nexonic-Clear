//! ISO base media files: MP4, MOV, M4A, HEIC and AVIF.
//!
//! Metadata boxes are removed and their containers rebuilt. Media data is
//! copied byte for byte; every absolute offset (chunk offsets, HEIF item
//! locations, fragment index) is moved by a map from old to new positions.
//! The fingerprint hashes the media samples found through the rewritten
//! offsets, which proves both that the media is unchanged and that the offsets
//! were fixed correctly.

use crate::stream::{digest_range, Mem, Plan, Src};
use crate::{details, malformed, Block, CleanOptions, Digest, Field, Kind, Parsed, Result};
use std::collections::{BTreeMap, HashSet};

const C2PA_UUID: [u8; 16] = [
    0xd8, 0xfe, 0xc3, 0xd6, 0x1b, 0x0e, 0x48, 0x3c, 0x92, 0x97, 0x58, 0x28, 0x87, 0x7e, 0xc4, 0x81,
];
const XMP_UUID: [u8; 16] = [
    0xbe, 0x7a, 0xcf, 0xcb, 0x97, 0xa9, 0x42, 0xe8, 0x9c, 0x71, 0x99, 0x94, 0x91, 0xe3, 0xaf, 0xac,
];

/// (offset, length) ranges.
type Ranges = Vec<(u64, u64)>;

#[derive(Clone, Copy, Debug)]
struct Bx {
    typ: [u8; 4],
    at: u64,
    hdr: u64,
    len: u64,
    uuid: Option<[u8; 16]>,
}

impl Bx {
    fn body(&self) -> u64 {
        self.at + self.hdr
    }
    fn end(&self) -> u64 {
        self.at + self.len
    }
    fn is(&self, t: &[u8; 4]) -> bool {
        &self.typ == t
    }
}

fn be16(b: &[u8], i: usize) -> Result<u16> {
    crate::be16(b, i).ok_or_else(|| malformed("truncated box"))
}
fn be32(b: &[u8], i: usize) -> Result<u32> {
    crate::be32(b, i).ok_or_else(|| malformed("truncated box"))
}
fn be64(b: &[u8], i: usize) -> Result<u64> {
    Ok(u64::from_be_bytes(
        b.get(i..i + 8)
            .ok_or_else(|| malformed("truncated box"))?
            .try_into()
            .unwrap(),
    ))
}
fn uint(b: &[u8], i: usize, n: usize) -> Result<u64> {
    if n > 8 {
        return Err(malformed("bad field size"));
    }
    let s = b.get(i..i + n).ok_or_else(|| malformed("truncated box"))?;
    Ok(s.iter().fold(0u64, |a, &x| (a << 8) | x as u64))
}
fn put_uint(out: &mut Vec<u8>, v: u64, n: usize) -> Result<()> {
    if n > 8 {
        return Err(malformed("bad field size"));
    }
    if n < 8 && v >> (8 * n) != 0 {
        return Err(malformed("an offset no longer fits its field"));
    }
    for k in (0..n).rev() {
        out.push((v >> (8 * k)) as u8);
    }
    Ok(())
}

fn boxes(src: &dyn Src, start: u64, end: u64) -> Result<Vec<Bx>> {
    let mut v = Vec::new();
    let mut i = start;
    while i < end {
        if end - i < 8 {
            if src.bytes(i, end - i)?.iter().all(|&b| b == 0) {
                break;
            }
            return Err(malformed("truncated box header"));
        }
        let h = src.bytes(i, 8)?;
        let mut len = be32(&h, 0)? as u64;
        let typ: [u8; 4] = h[4..8].try_into().unwrap();
        let mut hdr = 8;
        if len == 1 {
            len = be64(&src.bytes(i + 8, 8)?, 0)?;
            hdr = 16;
        } else if len == 0 {
            len = end - i;
        }
        if len < hdr || i.checked_add(len).is_none_or(|e| e > end) {
            return Err(malformed("box runs past the end of its container"));
        }
        let uuid = if &typ == b"uuid" {
            if len < hdr + 16 {
                return Err(malformed("truncated uuid box"));
            }
            let u = src.bytes(i + hdr, 16)?;
            hdr += 16;
            Some(u.try_into().unwrap())
        } else {
            None
        };
        v.push(Bx {
            typ,
            at: i,
            hdr,
            len,
            uuid,
        });
        i += len;
    }
    Ok(v)
}

fn kids(buf: &[u8], b: &Bx) -> Result<Vec<Bx>> {
    boxes(&Mem(buf), b.body(), b.end())
}

fn find(buf: &[u8], b: &Bx, t: &[u8; 4]) -> Result<Option<Bx>> {
    Ok(kids(buf, b)?.into_iter().find(|c| c.is(t)))
}

fn path(buf: &[u8], b: &Bx, p: &[&[u8; 4]]) -> Result<Option<Bx>> {
    let mut cur = *b;
    for t in p {
        match find(buf, &cur, t)? {
            Some(c) => cur = c,
            None => return Ok(None),
        }
    }
    Ok(Some(cur))
}

fn wrap(typ: &[u8; 4], uuid: Option<&[u8; 16]>, payload: &[u8]) -> Vec<u8> {
    let extra = if uuid.is_some() { 16 } else { 0 };
    let n = payload.len() as u64 + 8 + extra;
    let mut o = Vec::with_capacity(n as usize + 8);
    if n <= u32::MAX as u64 {
        o.extend_from_slice(&(n as u32).to_be_bytes());
        o.extend_from_slice(typ);
    } else {
        o.extend_from_slice(&1u32.to_be_bytes());
        o.extend_from_slice(typ);
        o.extend_from_slice(&(n + 8).to_be_bytes());
    }
    if let Some(u) = uuid {
        o.extend_from_slice(u);
    }
    o.extend_from_slice(payload);
    o
}

/// Start of a `meta` box's children: ISO meta is a full box, QuickTime's is not.
fn meta_kids(buf: &[u8], b: &Bx) -> Result<Vec<Bx>> {
    let s = b.body();
    let s8 = s as usize;
    let full = buf.get(s8 + 8..s8 + 12) == Some(b"hdlr")
        || (buf.get(s8..s8 + 4) == Some(&[0, 0, 0, 0]) && buf.get(s8 + 4..s8 + 8) != Some(b"hdlr"));
    boxes(&Mem(buf), if full { s + 4 } else { s }, b.end())
}

fn name4(t: &[u8; 4]) -> String {
    if t[0] == 0xA9 {
        format!("©{}", String::from_utf8_lossy(&t[1..]))
    } else {
        String::from_utf8_lossy(t).trim().to_string()
    }
}

fn field(group: &str, name: &str, value: String) -> Field {
    Field {
        group: group.into(),
        name: name.into(),
        value,
        tag: None,
    }
}

fn atom_name(t: &[u8; 4]) -> String {
    match t {
        b"\xA9xyz" => "Location".into(),
        b"\xA9mak" => "Make".into(),
        b"\xA9mod" => "Model".into(),
        b"\xA9swr" | b"\xA9too" => "Software".into(),
        b"\xA9day" => "Date".into(),
        b"\xA9nam" => "Title".into(),
        b"\xA9ART" => "Artist".into(),
        b"aART" => "Album artist".into(),
        b"\xA9alb" => "Album".into(),
        b"\xA9cmt" => "Comment".into(),
        b"\xA9gen" => "Genre".into(),
        b"\xA9wrt" => "Composer".into(),
        b"cprt" => "Copyright".into(),
        _ => name4(t),
    }
}

fn key_name(k: &str) -> String {
    match k.rsplit('.').next().unwrap_or(k) {
        "ISO6709" => "Location".into(),
        "make" => "Make".into(),
        "model" => "Model".into(),
        "software" => "Software".into(),
        "creationdate" => "Creation date".into(),
        _ => k.into(),
    }
}

#[derive(Default)]
struct Acc {
    blocks: Vec<Block>,
    fields: Vec<Field>,
}

/// Reads `keys` + `ilst` (QuickTime mdta) or iTunes `ilst` into fields.
fn meta_fields(buf: &[u8], b: &Bx, acc: &mut Acc) -> Result<bool> {
    let ks = meta_kids(buf, b)?;
    let mut keys: Vec<String> = Vec::new();
    let mut has_keys = false;
    for k in &ks {
        if k.is(b"keys") {
            has_keys = true;
            let s = k.body() as usize;
            let n = be32(buf, s + 4)? as usize;
            let mut i = s + 8;
            for _ in 0..n.min(10_000) {
                let sz = be32(buf, i)? as usize;
                if sz < 8 || i + sz > k.end() as usize {
                    break;
                }
                keys.push(String::from_utf8_lossy(&buf[i + 8..i + sz]).into());
                i += sz;
            }
        }
    }
    for k in &ks {
        if !k.is(b"ilst") {
            continue;
        }
        for item in kids(buf, k)? {
            let name = if has_keys {
                let idx = u32::from_be_bytes(item.typ) as usize;
                keys.get(idx.wrapping_sub(1))
                    .map(|k| key_name(k))
                    .unwrap_or_else(|| idx.to_string())
            } else {
                atom_name(&item.typ)
            };
            if let Some(d) = find(buf, &item, b"data")? {
                let s = d.body() as usize;
                let ty = be32(buf, s)? & 0xFF_FFFF;
                let v = &buf[(s + 8).min(d.end() as usize)..d.end() as usize];
                let val = match ty {
                    1 => String::from_utf8_lossy(v).into(),
                    13 | 14 | 27 => "(image)".into(),
                    21 | 22 if v.len() <= 8 => uint(v, 0, v.len())?.to_string(),
                    _ => format!("({} bytes)", v.len()),
                };
                acc.fields.push(field("QuickTime", &name, val));
            }
        }
    }
    Ok(has_keys)
}

fn classify(
    parent: &[u8; 4],
    c: &Bx,
    buf: &[u8],
    acc: Option<&mut Acc>,
) -> Result<Option<(Kind, String)>> {
    if c.uuid == Some(C2PA_UUID) {
        return Ok(Some((Kind::C2pa, "C2PA manifest (uuid box)".into())));
    }
    if c.uuid == Some(XMP_UUID) {
        if let Some(a) = acc {
            a.fields
                .extend(details::xmp(&buf[c.body() as usize..c.end() as usize]));
        }
        return Ok(Some((Kind::Xmp, "XMP (uuid box)".into())));
    }
    if c.is(b"XMP_") {
        if let Some(a) = acc {
            a.fields
                .extend(details::xmp(&buf[c.body() as usize..c.end() as usize]));
        }
        return Ok(Some((Kind::Xmp, "XMP_ box".into())));
    }
    Ok(match (parent, &c.typ) {
        (b"moov" | b"trak" | b"udta", b"meta") => {
            let mut scratch = Acc::default();
            let a = acc.unwrap_or(&mut scratch);
            if meta_fields(buf, c, a)? {
                Some((
                    Kind::Exif,
                    "QuickTime metadata (make, model, location)".into(),
                ))
            } else if parent == b"udta" {
                Some((Kind::Comment, "iTunes tags".into()))
            } else {
                Some((Kind::Exif, "QuickTime metadata".into()))
            }
        }
        (b"udta", t) => {
            let p = c.body() as usize;
            let text = || -> Option<String> {
                let n = crate::be16(buf, p)? as usize;
                let s = buf.get(p + 4..p + 4 + n)?;
                Some(String::from_utf8_lossy(s).into())
            };
            let kind = match t {
                b"\xA9xyz" | b"\xA9mak" | b"\xA9mod" | b"\xA9swr" | b"\xA9day" => Kind::Exif,
                _ if t[0] == 0xA9 => Kind::Comment,
                _ => Kind::Other,
            };
            if let (Some(a), Some(v), true) = (acc, text(), t[0] == 0xA9) {
                a.fields.push(field("QuickTime", &atom_name(t), v));
            }
            Some((kind, format!("udta {}", name4(t))))
        }
        (b"moov" | b"trak", b"uuid") => Some((Kind::Other, "uuid box".into())),
        _ => None,
    })
}

/// Walks moov/trak/udta. Records blocks when `acc` is set; rebuilds the box
/// without removed children when `o` is set. Returns None if the box ends up empty
/// and may be dropped.
fn walk(
    buf: &[u8],
    b: &Bx,
    o: Option<&CleanOptions>,
    acc: &mut Option<&mut Acc>,
) -> Result<Option<Vec<u8>>> {
    let mut out = Vec::new();
    let mut removed_any = false;
    for c in kids(buf, b)? {
        let cls = classify(&b.typ, &c, buf, acc.as_deref_mut())?;
        match cls {
            Some((kind, label)) => {
                if let Some(a) = acc.as_deref_mut() {
                    a.blocks.push(Block {
                        kind,
                        label,
                        size: c.len as usize,
                    });
                }
                if o.is_some_and(|o| o.removes(kind)) {
                    removed_any = true;
                } else {
                    out.extend_from_slice(&buf[c.at as usize..c.end() as usize]);
                }
            }
            None => {
                let descend = matches!(
                    (&b.typ, &c.typ),
                    (b"moov", b"trak" | b"udta") | (b"trak", b"udta")
                );
                if descend {
                    match walk(buf, &c, o, acc)? {
                        Some(v) => out.extend_from_slice(&v),
                        None => removed_any = true,
                    }
                } else {
                    out.extend_from_slice(&buf[c.at as usize..c.end() as usize]);
                }
            }
        }
    }
    if o.is_some() && b.is(b"udta") && out.is_empty() && removed_any {
        return Ok(None);
    }
    if o.is_none() {
        return Ok(Some(Vec::new()));
    }
    Ok(Some(wrap(&b.typ, b.uuid.as_ref(), &out)))
}

fn classify_top(b: &Bx) -> Option<(Kind, String)> {
    match (&b.typ, b.uuid) {
        (_, Some(C2PA_UUID)) => Some((Kind::C2pa, "C2PA manifest (uuid box)".into())),
        (_, Some(XMP_UUID)) => Some((Kind::Xmp, "XMP (uuid box)".into())),
        (b"uuid", _) => Some((Kind::Other, "uuid box".into())),
        (b"XMP_", _) => Some((Kind::Xmp, "XMP_ box".into())),
        _ => None,
    }
}

// ---------- movies ----------

fn sample_sizes(buf: &[u8], stbl: &Bx) -> Result<(u32, Vec<u32>)> {
    if let Some(z) = find(buf, stbl, b"stsz")? {
        let s = z.body() as usize;
        let size = be32(buf, s + 4)?;
        let n = be32(buf, s + 8)? as usize;
        if size != 0 {
            return Ok((size, vec![size; n.min(1 << 26)]));
        }
        if s + 12 + 4 * n > z.end() as usize {
            return Err(malformed("truncated sample size table"));
        }
        return Ok((
            0,
            (0..n).map(|k| be32(buf, s + 12 + 4 * k).unwrap()).collect(),
        ));
    }
    if let Some(z) = find(buf, stbl, b"stz2")? {
        let s = z.body() as usize;
        let fs = buf.get(s + 7).copied().unwrap_or(0) as usize;
        let n = be32(buf, s + 8)? as usize;
        let mut v = Vec::with_capacity(n.min(1 << 24));
        for k in 0..n {
            v.push(match fs {
                4 => {
                    let byte = *buf
                        .get(s + 12 + k / 2)
                        .ok_or_else(|| malformed("truncated stz2"))?;
                    (if k % 2 == 0 { byte >> 4 } else { byte & 15 }) as u32
                }
                8 => *buf
                    .get(s + 12 + k)
                    .ok_or_else(|| malformed("truncated stz2"))? as u32,
                16 => be16(buf, s + 12 + 2 * k)? as u32,
                _ => return Err(malformed("bad stz2 field size")),
            });
        }
        return Ok((0, v));
    }
    Ok((0, Vec::new()))
}

/// (offset, size) of every chunk in a track.
fn chunks(buf: &[u8], stbl: &Bx) -> Result<Vec<(u64, u64)>> {
    let mut offs: Vec<u64> = Vec::new();
    if let Some(c) = find(buf, stbl, b"stco")? {
        let s = c.body() as usize;
        let n = be32(buf, s + 4)? as usize;
        for k in 0..n {
            offs.push(be32(buf, s + 8 + 4 * k)? as u64);
        }
    } else if let Some(c) = find(buf, stbl, b"co64")? {
        let s = c.body() as usize;
        let n = be32(buf, s + 4)? as usize;
        for k in 0..n {
            offs.push(be64(buf, s + 8 + 8 * k)?);
        }
    }
    if offs.is_empty() {
        return Ok(Vec::new());
    }
    let (_, sizes) = sample_sizes(buf, stbl)?;
    let mut stsc: Vec<(u32, u32)> = Vec::new();
    if let Some(c) = find(buf, stbl, b"stsc")? {
        let s = c.body() as usize;
        let n = be32(buf, s + 4)? as usize;
        for k in 0..n {
            stsc.push((be32(buf, s + 8 + 12 * k)?, be32(buf, s + 12 + 12 * k)?));
        }
    }
    let mut out = Vec::with_capacity(offs.len());
    let mut sample = 0usize;
    for (ci, off) in offs.iter().enumerate() {
        let c = ci as u32 + 1;
        let spc = stsc.iter().rev().find(|e| e.0 <= c).map_or(0, |e| e.1) as usize;
        let end = sample.checked_add(spc).filter(|&e| e <= sizes.len());
        let Some(end) = end else {
            return Err(malformed("sample tables do not match"));
        };
        let size: u64 = sizes[sample..end].iter().map(|&x| x as u64).sum();
        sample = end;
        out.push((*off, size));
    }
    Ok(out)
}

/// Dimensions, duration and the media fingerprint for every track.
fn movie_info(buf: &[u8], moov: &Bx, src: &dyn Src, p: &mut Parsed, fp: &mut Digest) -> Result<()> {
    if let Some(m) = find(buf, moov, b"mvhd")? {
        let s = m.body() as usize;
        let (ts, dur) = if buf.get(s) == Some(&1) {
            (be32(buf, s + 20)?, be64(buf, s + 24)?)
        } else {
            (be32(buf, s + 12)?, be32(buf, s + 16)? as u64)
        };
        if ts > 0 {
            p.fields.push(field(
                "Media",
                "Duration",
                format!("{:.2} s", dur as f64 / ts as f64),
            ));
        }
    }
    for t in kids(buf, moov)?.into_iter().filter(|c| c.is(b"trak")) {
        if let Some(h) = find(buf, &t, b"tkhd")? {
            let e = h.end() as usize;
            let w = be32(buf, e - 8)? >> 16;
            let hh = be32(buf, e - 4)? >> 16;
            if p.width == 0 && w > 0 && hh > 0 {
                p.width = w;
                p.height = hh;
            }
        }
        if let Some(h) = path(buf, &t, &[b"mdia", b"hdlr"])? {
            fp.add(
                buf.get(h.body() as usize + 8..h.body() as usize + 12)
                    .unwrap_or_default(),
            );
        }
        let Some(stbl) = path(buf, &t, &[b"mdia", b"minf", b"stbl"])? else {
            continue;
        };
        for c in kids(buf, &stbl)? {
            if matches!(
                &c.typ,
                b"stsd" | b"stts" | b"ctts" | b"stsc" | b"stsz" | b"stz2" | b"stss" | b"sdtp"
            ) {
                fp.add(&buf[c.at as usize..c.end() as usize]);
            }
        }
        for (off, size) in chunks(buf, &stbl)? {
            digest_range(fp, src, off, size)?;
        }
    }
    Ok(())
}

// ---------- HEIF / AVIF ----------

#[derive(Clone, Debug)]
struct Loc {
    id: u32,
    method: u8,
    dref: u16,
    base: u64,
    ext: Vec<(u64, u64, u64)>, // (index, offset, length)
}

struct Heif {
    buf: Vec<u8>,
    meta: Bx,
    /// item id -> (type, content type, infe box)
    items: BTreeMap<u32, ([u8; 4], String, Bx)>,
    iinf: Option<Bx>,
    iloc: Option<(Bx, u8, [usize; 4])>,
    locs: Vec<Loc>,
    idat: Option<Bx>,
    pitm: u32,
}

fn heif_kind(typ: &[u8; 4], ct: &str) -> Option<(Kind, String)> {
    match typ {
        b"Exif" => Some((Kind::Exif, "Exif item".into())),
        b"mime" if ct.contains("rdf+xml") || ct.contains("xmp") => {
            Some((Kind::Xmp, "XMP item".into()))
        }
        b"mime" | b"uri " => Some((Kind::Other, format!("Metadata item ({ct})"))),
        _ => None,
    }
}

fn heif_parse(buf: Vec<u8>) -> Result<Heif> {
    let meta = boxes(&Mem(&buf), 0, buf.len() as u64)?[0];
    let mut h = Heif {
        meta,
        items: BTreeMap::new(),
        iinf: None,
        iloc: None,
        locs: Vec::new(),
        idat: None,
        pitm: 0,
        buf: Vec::new(),
    };
    let ks = boxes(&Mem(&buf), meta.body() + 4, meta.end())?;
    for c in &ks {
        let s = c.body() as usize;
        match &c.typ {
            b"pitm" => {
                h.pitm = if buf[s] == 0 {
                    be16(&buf, s + 4)? as u32
                } else {
                    be32(&buf, s + 4)?
                }
            }
            b"idat" => h.idat = Some(*c),
            b"iinf" => {
                h.iinf = Some(*c);
                let first = if buf[s] == 0 { s + 6 } else { s + 8 };
                for e in boxes(&Mem(&buf), first as u64, c.end())?
                    .into_iter()
                    .filter(|e| e.is(b"infe"))
                {
                    let es = e.body() as usize;
                    let v = buf[es];
                    if v < 2 {
                        continue;
                    }
                    let (id, ti) = if v == 2 {
                        (be16(&buf, es + 4)? as u32, es + 8)
                    } else {
                        (be32(&buf, es + 4)?, es + 10)
                    };
                    let typ: [u8; 4] = buf
                        .get(ti..ti + 4)
                        .ok_or_else(|| malformed("truncated infe"))?
                        .try_into()
                        .unwrap();
                    let mut ct = String::new();
                    if &typ == b"mime" {
                        let rest = &buf[ti + 4..e.end() as usize];
                        let name_end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
                        let r2 = rest.get(name_end + 1..).unwrap_or_default();
                        ct = String::from_utf8_lossy(
                            &r2[..r2.iter().position(|&b| b == 0).unwrap_or(r2.len())],
                        )
                        .into();
                    }
                    h.items.insert(id, (typ, ct, e));
                }
            }
            b"iloc" => {
                let v = buf[s];
                let a = buf
                    .get(s + 4)
                    .copied()
                    .ok_or_else(|| malformed("truncated iloc"))?;
                let b2 = buf
                    .get(s + 5)
                    .copied()
                    .ok_or_else(|| malformed("truncated iloc"))?;
                let sizes = [
                    (a >> 4) as usize,
                    (a & 15) as usize,
                    (b2 >> 4) as usize,
                    if v > 0 { (b2 & 15) as usize } else { 0 },
                ];
                if sizes.iter().any(|&z| !matches!(z, 0 | 4 | 8)) || v > 2 {
                    return Err(malformed("bad iloc field sizes"));
                }
                let mut i = s + 6;
                let n = if v < 2 {
                    let n = be16(&buf, i)? as usize;
                    i += 2;
                    n
                } else {
                    let n = be32(&buf, i)? as usize;
                    i += 4;
                    n
                };
                for _ in 0..n {
                    let id = if v < 2 {
                        let x = be16(&buf, i)? as u32;
                        i += 2;
                        x
                    } else {
                        let x = be32(&buf, i)?;
                        i += 4;
                        x
                    };
                    let method = if v > 0 {
                        let m = (be16(&buf, i)? & 15) as u8;
                        i += 2;
                        m
                    } else {
                        0
                    };
                    let dref = be16(&buf, i)?;
                    i += 2;
                    let base = uint(&buf, i, sizes[2])?;
                    i += sizes[2];
                    let ne = be16(&buf, i)? as usize;
                    i += 2;
                    let mut ext = Vec::new();
                    for _ in 0..ne {
                        let idx = uint(&buf, i, sizes[3])?;
                        i += sizes[3];
                        let off = uint(&buf, i, sizes[0])?;
                        i += sizes[0];
                        let len = uint(&buf, i, sizes[1])?;
                        i += sizes[1];
                        ext.push((idx, off, len));
                    }
                    h.locs.push(Loc {
                        id,
                        method,
                        dref,
                        base,
                        ext,
                    });
                }
                h.iloc = Some((*c, v, sizes));
            }
            _ => {}
        }
    }
    h.buf = buf;
    Ok(h)
}

impl Heif {
    fn loc(&self, id: u32) -> Option<&Loc> {
        self.locs.iter().find(|l| l.id == id)
    }
    /// Reads an item's bytes (bounded) from the file or idat.
    fn data(&self, id: u32, src: &dyn Src, max: u64) -> Result<Vec<u8>> {
        let mut v = Vec::new();
        let Some(l) = self.loc(id) else { return Ok(v) };
        for &(_, off, len) in &l.ext {
            let at = l.base + off;
            let len = if len == 0 { max } else { len.min(max) };
            match l.method {
                0 => v.extend(src.bytes(at, len.min(src.len().saturating_sub(at)))?),
                1 => {
                    let idat = self.idat.ok_or_else(|| malformed("item in missing idat"))?;
                    let s = (idat.body() + at) as usize;
                    let e = (s as u64 + len).min(idat.end()) as usize;
                    v.extend_from_slice(
                        self.buf
                            .get(s..e)
                            .ok_or_else(|| malformed("item outside idat"))?,
                    );
                }
                _ => {}
            }
        }
        Ok(v)
    }
    fn fingerprint(&self, src: &dyn Src, fp: &mut Digest) -> Result<()> {
        let ks = boxes(&Mem(&self.buf), self.meta.body() + 4, self.meta.end())?;
        fp.add(&self.pitm.to_be_bytes());
        if let Some(iprp) = ks.iter().find(|c| c.is(b"iprp")) {
            for c in kids(&self.buf, iprp)? {
                if c.is(b"ipco") {
                    fp.add(&self.buf[c.at as usize..c.end() as usize]);
                }
            }
        }
        let props = self.ipma()?;
        for (id, (typ, ct, _)) in &self.items {
            if heif_kind(typ, ct).is_some() {
                continue;
            }
            fp.add(&id.to_be_bytes());
            fp.add(typ);
            fp.add(props.get(id).map(|v| v.as_slice()).unwrap_or_default());
            let Some(l) = self.loc(*id) else { continue };
            for &(_, off, len) in &l.ext {
                let at = l.base + off;
                match l.method {
                    0 => {
                        let len = if len == 0 {
                            src.len().saturating_sub(at)
                        } else {
                            len
                        };
                        digest_range(fp, src, at, len)?
                    }
                    1 => fp.add(&self.data_ext(at, len)?),
                    _ => {}
                }
            }
        }
        Ok(())
    }
    fn data_ext(&self, at: u64, len: u64) -> Result<Vec<u8>> {
        let idat = self.idat.ok_or_else(|| malformed("item in missing idat"))?;
        let s = (idat.body() + at) as usize;
        let e = if len == 0 {
            idat.end() as usize
        } else {
            s + len as usize
        };
        Ok(self
            .buf
            .get(s..e.min(idat.end() as usize))
            .ok_or_else(|| malformed("item outside idat"))?
            .to_vec())
    }
    /// Raw property association bytes per item.
    fn ipma(&self) -> Result<BTreeMap<u32, Vec<u8>>> {
        let mut m = BTreeMap::new();
        for (ipma, v, flags) in self.ipma_boxes()? {
            let mut i = ipma.body() as usize + 4;
            let n = be32(&self.buf, i)? as usize;
            i += 4;
            for _ in 0..n {
                let id = if v < 1 {
                    let x = be16(&self.buf, i)? as u32;
                    i += 2;
                    x
                } else {
                    let x = be32(&self.buf, i)?;
                    i += 4;
                    x
                };
                let cnt = *self.buf.get(i).ok_or_else(|| malformed("truncated ipma"))? as usize;
                let w = if flags & 1 != 0 { 2 } else { 1 };
                let e = i + 1 + cnt * w;
                m.insert(
                    id,
                    self.buf
                        .get(i..e)
                        .ok_or_else(|| malformed("truncated ipma"))?
                        .to_vec(),
                );
                i = e;
            }
        }
        Ok(m)
    }
    fn ipma_boxes(&self) -> Result<Vec<(Bx, u8, u32)>> {
        let ks = boxes(&Mem(&self.buf), self.meta.body() + 4, self.meta.end())?;
        let mut v = Vec::new();
        if let Some(iprp) = ks.iter().find(|c| c.is(b"iprp")) {
            for c in kids(&self.buf, iprp)?.into_iter().filter(|c| c.is(b"ipma")) {
                let s = c.body() as usize;
                v.push((c, self.buf[s], be32(&self.buf, s)? & 0xFF_FFFF));
            }
        }
        Ok(v)
    }
    fn dims(&self) -> Result<(u32, u32, bool)> {
        let ks = boxes(&Mem(&self.buf), self.meta.body() + 4, self.meta.end())?;
        let Some(iprp) = ks.iter().find(|c| c.is(b"iprp")) else {
            return Ok((0, 0, false));
        };
        let Some(ipco) = find(&self.buf, iprp, b"ipco")? else {
            return Ok((0, 0, false));
        };
        let props = kids(&self.buf, &ipco)?;
        let alpha = props.iter().any(|p| {
            p.is(b"auxC")
                && self.buf[p.body() as usize..p.end() as usize]
                    .windows(5)
                    .any(|w| w == b"alpha")
        });
        let map = self.ipma()?;
        let wide = self.ipma_boxes()?.first().is_some_and(|b| b.2 & 1 != 0);
        if let Some(a) = map.get(&self.pitm) {
            let cnt = a[0] as usize;
            for k in 0..cnt {
                let idx = if wide {
                    (be16(a, 1 + 2 * k)? & 0x7FFF) as usize
                } else {
                    (a[1 + k] & 0x7F) as usize
                };
                if let Some(p) = idx.checked_sub(1).and_then(|i| props.get(i)) {
                    if p.is(b"ispe") {
                        let s = p.body() as usize;
                        return Ok((be32(&self.buf, s + 4)?, be32(&self.buf, s + 8)?, alpha));
                    }
                }
            }
        }
        Ok((0, 0, alpha))
    }

    fn removed(&self, o: &CleanOptions) -> HashSet<u32> {
        self.items
            .iter()
            .filter(|(_, (t, ct, _))| heif_kind(t, ct).is_some_and(|(k, _)| o.removes(k)))
            .map(|(id, _)| *id)
            .collect()
    }

    /// File ranges (method 0) and idat ranges (method 1) to cut.
    fn cuts(&self, remove: &HashSet<u32>) -> Result<(Ranges, Ranges)> {
        let (mut file, mut idat) = (Vec::new(), Vec::new());
        for l in self.locs.iter().filter(|l| remove.contains(&l.id)) {
            for &(_, off, len) in &l.ext {
                if len == 0 {
                    return Err(malformed("a metadata item has no length"));
                }
                match l.method {
                    0 if l.dref == 0 => file.push((l.base + off, len)),
                    1 => idat.push((l.base + off, len)),
                    _ => {}
                }
            }
        }
        file.sort();
        idat.sort();
        Ok((file, idat))
    }

    /// Rebuilds the meta box without removed items. `map` moves absolute file
    /// offsets. The length does not depend on `map`.
    fn build(
        &self,
        remove: &HashSet<u32>,
        idat_cuts: &[(u64, u64)],
        map: &dyn Fn(u64) -> Option<u64>,
    ) -> Result<Vec<u8>> {
        let b = &self.buf;
        let ks = boxes(&Mem(b), self.meta.body() + 4, self.meta.end())?;
        let mut out = b[self.meta.body() as usize..self.meta.body() as usize + 4].to_vec();
        let rel = |x: u64| -> Option<u64> {
            let mut shift = 0;
            for &(a, l) in idat_cuts {
                if x >= a + l {
                    shift += l;
                } else if x >= a {
                    return None;
                }
            }
            Some(x - shift)
        };
        for c in &ks {
            let s = c.body() as usize;
            match &c.typ {
                b"iinf" => {
                    let v = b[s];
                    let first = if v == 0 { s + 6 } else { s + 8 };
                    let mut body = b[s..s + 4].to_vec();
                    let mut entries = Vec::new();
                    let mut n = 0u32;
                    for e in boxes(&Mem(b), first as u64, c.end())? {
                        let keep = !(e.is(b"infe")
                            && self
                                .items
                                .iter()
                                .any(|(id, it)| it.2.at == e.at && remove.contains(id)));
                        if keep {
                            entries.extend_from_slice(&b[e.at as usize..e.end() as usize]);
                            n += 1;
                        }
                    }
                    if v == 0 {
                        body.extend_from_slice(&(n as u16).to_be_bytes());
                    } else {
                        body.extend_from_slice(&n.to_be_bytes());
                    }
                    body.extend(entries);
                    out.extend(wrap(b"iinf", None, &body));
                }
                b"iloc" => {
                    let (_, v, sz) = self.iloc.unwrap();
                    let mut body = b[s..s + 6].to_vec();
                    let kept: Vec<&Loc> = self
                        .locs
                        .iter()
                        .filter(|l| !remove.contains(&l.id))
                        .collect();
                    if v < 2 {
                        body.extend_from_slice(&(kept.len() as u16).to_be_bytes());
                    } else {
                        body.extend_from_slice(&(kept.len() as u32).to_be_bytes());
                    }
                    for l in kept {
                        if v < 2 {
                            body.extend_from_slice(&(l.id as u16).to_be_bytes());
                        } else {
                            body.extend_from_slice(&l.id.to_be_bytes());
                        }
                        if v > 0 {
                            body.extend_from_slice(&(l.method as u16).to_be_bytes());
                        }
                        body.extend_from_slice(&l.dref.to_be_bytes());
                        let mover: &dyn Fn(u64) -> Option<u64> = match (l.method, l.dref) {
                            (0, 0) => map,
                            (1, _) => &rel,
                            _ => &|x| Some(x),
                        };
                        let first = l.ext.iter().map(|e| l.base + e.1).min();
                        let new_base = if let (true, Some(f)) = (sz[2] > 0, first) {
                            mover(f)
                                .ok_or_else(|| malformed("item data overlaps removed metadata"))?
                        } else {
                            l.base
                        };
                        put_uint(&mut body, new_base, sz[2])?;
                        body.extend_from_slice(&(l.ext.len() as u16).to_be_bytes());
                        for &(idx, off, len) in &l.ext {
                            put_uint(&mut body, idx, sz[3])?;
                            let abs = mover(l.base + off)
                                .ok_or_else(|| malformed("item data overlaps removed metadata"))?;
                            if len > 0 && mover(l.base + off + len - 1) != Some(abs + len - 1) {
                                return Err(malformed("item data overlaps removed metadata"));
                            }
                            let o2 = abs
                                .checked_sub(new_base)
                                .ok_or_else(|| malformed("item data moved before its base"))?;
                            let o2 = if sz[2] > 0 { o2 } else { abs };
                            put_uint(&mut body, o2, sz[0])?;
                            put_uint(&mut body, len, sz[1])?;
                        }
                    }
                    out.extend(wrap(b"iloc", None, &body));
                }
                b"iref" => {
                    let v = b[s];
                    let w = if v == 0 { 2 } else { 4 };
                    let mut body = b[s..s + 4].to_vec();
                    for r in boxes(&Mem(b), s as u64 + 4, c.end())? {
                        let mut i = r.body() as usize;
                        let from = uint(b, i, w)? as u32;
                        i += w;
                        let n = be16(b, i)? as usize;
                        i += 2;
                        let mut to = Vec::new();
                        for _ in 0..n {
                            let t = uint(b, i, w)? as u32;
                            i += w;
                            if !remove.contains(&t) {
                                to.push(t);
                            }
                        }
                        if remove.contains(&from) || to.is_empty() {
                            continue;
                        }
                        let mut e = Vec::new();
                        put_uint(&mut e, from as u64, w)?;
                        e.extend_from_slice(&(to.len() as u16).to_be_bytes());
                        for t in to {
                            put_uint(&mut e, t as u64, w)?;
                        }
                        body.extend(wrap(&r.typ, None, &e));
                    }
                    if body.len() > 4 {
                        out.extend(wrap(b"iref", None, &body));
                    }
                }
                b"iprp" => {
                    let mut body = Vec::new();
                    for p in kids(b, c)? {
                        if !p.is(b"ipma") {
                            body.extend_from_slice(&b[p.at as usize..p.end() as usize]);
                            continue;
                        }
                        let ps = p.body() as usize;
                        let v = b[ps];
                        let flags = be32(b, ps)? & 0xFF_FFFF;
                        let wdt = if flags & 1 != 0 { 2 } else { 1 };
                        let mut i = ps + 8;
                        let n = be32(b, ps + 4)? as usize;
                        let mut entries = Vec::new();
                        let mut kept = 0u32;
                        for _ in 0..n {
                            let st = i;
                            let id = if v < 1 {
                                let x = be16(b, i)? as u32;
                                i += 2;
                                x
                            } else {
                                let x = be32(b, i)?;
                                i += 4;
                                x
                            };
                            let cnt =
                                *b.get(i).ok_or_else(|| malformed("truncated ipma"))? as usize;
                            i += 1 + cnt * wdt;
                            if !remove.contains(&id) {
                                entries.extend_from_slice(
                                    b.get(st..i).ok_or_else(|| malformed("truncated ipma"))?,
                                );
                                kept += 1;
                            }
                        }
                        let mut e = b[ps..ps + 4].to_vec();
                        e.extend_from_slice(&kept.to_be_bytes());
                        e.extend(entries);
                        body.extend(wrap(b"ipma", None, &e));
                    }
                    out.extend(wrap(b"iprp", None, &body));
                }
                b"idat" => {
                    let data = &b[s..c.end() as usize];
                    let mut kept = Vec::with_capacity(data.len());
                    let mut i = 0u64;
                    for &(a, l) in idat_cuts {
                        kept.extend_from_slice(
                            data.get(i as usize..a as usize)
                                .ok_or_else(|| malformed("bad idat range"))?,
                        );
                        i = a + l;
                    }
                    kept.extend_from_slice(data.get(i as usize..).unwrap_or_default());
                    out.extend(wrap(b"idat", None, &kept));
                }
                _ => out.extend_from_slice(&b[c.at as usize..c.end() as usize]),
            }
        }
        Ok(wrap(b"meta", None, &out))
    }
}

fn is_pict_meta(src: &dyn Src, b: &Bx) -> Result<bool> {
    if !b.is(b"meta") || b.len < b.hdr + 24 {
        return Ok(false);
    }
    let h = src.bytes(b.body() + 4, 20)?;
    Ok(&h[4..8] == b"hdlr" && &h[16..20] == b"pict")
}

// ---------- entry points ----------

pub(crate) fn parse(src: &dyn Src) -> Result<Parsed> {
    let tops = boxes(src, 0, src.len())?;
    if !tops.first().is_some_and(|b| b.is(b"ftyp")) {
        return Err(malformed("missing ftyp box"));
    }
    let mut p = Parsed::empty();
    let mut acc = Acc::default();
    let mut fp = Digest::new();
    let fragmented = tops.iter().any(|b| b.is(b"moof"));
    let mut has_media = false;
    for b in &tops {
        if b.is(b"moov") {
            has_media = true;
            let buf = src.bytes(b.at, b.len)?;
            let mb = Bx { at: 0, ..*b };
            walk(&buf, &mb, None, &mut Some(&mut acc))?;
            movie_info(&buf, &mb, src, &mut p, &mut fp)?;
        } else if is_pict_meta(src, b)? {
            has_media = true;
            let h = heif_parse(src.bytes(b.at, b.len)?)?;
            let (w, hh, alpha) = h.dims()?;
            if w > 0 {
                (p.width, p.height) = (w, hh);
            }
            p.has_alpha = alpha;
            for (id, (typ, ct, _)) in &h.items {
                let Some((kind, label)) = heif_kind(typ, ct) else {
                    continue;
                };
                let size: u64 = h.loc(*id).map_or(0, |l| l.ext.iter().map(|e| e.2).sum());
                let data = h.data(*id, src, 4 << 20)?;
                if kind == Kind::Exif && p.exif_tiff.is_none() && data.len() > 4 {
                    let skip = 4 + u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
                    if let Some(t) = data.get(skip..) {
                        p.exif_tiff = Some(t.to_vec());
                    }
                }
                if kind == Kind::Xmp {
                    acc.fields.extend(details::xmp(&data));
                }
                acc.blocks.push(Block {
                    kind,
                    label,
                    size: size as usize,
                });
            }
            h.fingerprint(src, &mut fp)?;
        } else if fragmented && (b.is(b"moof") || b.is(b"mdat")) {
            digest_range(&mut fp, src, b.body(), b.len - b.hdr)?;
        } else if let Some((kind, label)) = classify_top(b) {
            if kind == Kind::Xmp && b.len - b.hdr <= 4 << 20 {
                acc.fields
                    .extend(details::xmp(&src.bytes(b.body(), b.len - b.hdr)?));
            }
            acc.blocks.push(Block {
                kind,
                label,
                size: b.len as usize,
            });
        }
    }
    if !has_media {
        return Err(malformed("no movie or image metadata box"));
    }
    if fragmented {
        let (frozen_until, _) = frozen(src, &tops)?;
        if frozen_until > 0 {
            p.warnings.push("This fragmented video uses absolute offsets, so metadata stored before its last fragment can't be removed.".into());
        } else {
            p.warnings
                .push("This is a fragmented video. Metadata inside the fragments is kept.".into());
        }
    }
    p.blocks = acc.blocks;
    p.fields.extend(acc.fields);
    p.fingerprint = fp.finish();
    Ok(p)
}

/// (end of the region that must not move, start of the region that must not change size).
fn frozen(src: &dyn Src, tops: &[Bx]) -> Result<(u64, u64)> {
    let mut until = 0;
    let mut explicit = false;
    let mut last = 0;
    for b in tops.iter().filter(|b| b.is(b"moof")) {
        let buf = src.bytes(b.at, b.len)?;
        let mb = Bx { at: 0, ..*b };
        for traf in kids(&buf, &mb)?.into_iter().filter(|c| c.is(b"traf")) {
            if let Some(t) = find(&buf, &traf, b"tfhd")? {
                if be32(&buf, t.body() as usize)? & 1 != 0 {
                    explicit = true;
                }
            }
        }
        last = b.end();
    }
    if explicit {
        until = tops
            .iter()
            .filter(|b| b.at >= last && b.is(b"mdat"))
            .map(|b| b.end())
            .next()
            .unwrap_or(last);
    }
    let from = tops
        .iter()
        .find(|b| b.is(b"sidx"))
        .map_or(u64::MAX, |b| b.at);
    Ok((until, from))
}

enum Out {
    Copy(u64, u64),
    Bytes(Vec<u8>),
    Moov(Vec<u8>),
    Meta(usize),
    Mfra(Vec<u8>),
}

pub(crate) fn clean(src: &dyn Src, o: &CleanOptions) -> Result<Plan> {
    let tops = boxes(src, 0, src.len())?;
    if !tops.first().is_some_and(|b| b.is(b"ftyp")) {
        return Err(malformed("missing ftyp box"));
    }
    let (until, from) = frozen(src, &tops)?;
    let mut outs: Vec<Out> = Vec::new();
    let mut heifs: Vec<(Heif, HashSet<u32>, Ranges)> = Vec::new();
    let mut file_cuts: Vec<(u64, u64)> = Vec::new();
    for b in &tops {
        let fixed = b.at < until || b.at >= from;
        if b.is(b"moov") && !fixed {
            let buf = src.bytes(b.at, b.len)?;
            let mb = Bx { at: 0, ..*b };
            let v = walk(&buf, &mb, Some(o), &mut None)?.unwrap_or_default();
            outs.push(Out::Moov(v));
        } else if b.is(b"moov") {
            outs.push(Out::Moov(src.bytes(b.at, b.len)?));
        } else if is_pict_meta(src, b)? && !fixed {
            let h = heif_parse(src.bytes(b.at, b.len)?)?;
            let rm = h.removed(o);
            let (fc, ic) = h.cuts(&rm)?;
            file_cuts.extend(fc);
            heifs.push((h, rm, ic));
            outs.push(Out::Meta(heifs.len() - 1));
        } else if b.is(b"mfra") && b.len <= 16 << 20 {
            outs.push(Out::Mfra(src.bytes(b.at, b.len)?));
        } else if classify_top(b).is_some_and(|(k, _)| o.removes(k)) && !fixed {
        } else {
            outs.push(Out::Copy(b.at, b.len));
        }
    }
    // Cut removed HEIF item data out of the boxes that hold it.
    file_cuts.sort();
    if !file_cuts.is_empty() {
        let mut next = Vec::new();
        for out in outs {
            let Out::Copy(at, len) = out else {
                next.push(out);
                continue;
            };
            let inside: Vec<(u64, u64)> = file_cuts
                .iter()
                .copied()
                .filter(|&(a, l)| a >= at && a + l <= at + len)
                .collect();
            if inside.is_empty() {
                next.push(Out::Copy(at, len));
                continue;
            }
            let b = tops.iter().find(|b| b.at == at).copied().unwrap();
            if !b.is(b"mdat") || inside.iter().any(|&(a, _)| a < b.body()) {
                return Err(malformed("metadata item stored outside the media data box"));
            }
            let removed: u64 = inside.iter().map(|c| c.1).sum();
            let payload = b.len - b.hdr - removed;
            let mut h = wrap(b"mdat", None, &[]);
            if payload + 8 > u32::MAX as u64 {
                h = vec![0, 0, 0, 1, b'm', b'd', b'a', b't'];
                h.extend_from_slice(&(payload + 16).to_be_bytes());
            } else {
                h[0..4].copy_from_slice(&((payload + 8) as u32).to_be_bytes());
            }
            next.push(Out::Bytes(h));
            let mut i = b.body();
            for (a, l) in inside {
                if a < i {
                    return Err(malformed("overlapping metadata items"));
                }
                next.push(Out::Copy(i, a - i));
                i = a + l;
            }
            next.push(Out::Copy(i, b.end() - i));
        }
        outs = next;
    }
    // Lengths are final; build the old -> new offset map from copied ranges.
    let identity = |x: u64| Some(x);
    let mut lens = Vec::new();
    for out in &outs {
        lens.push(match out {
            Out::Copy(_, l) => *l,
            Out::Bytes(v) | Out::Moov(v) | Out::Mfra(v) => v.len() as u64,
            Out::Meta(i) => {
                let (h, rm, ic) = &heifs[*i];
                h.build(rm, ic, &identity)?.len() as u64
            }
        });
    }
    let mut spans: Vec<(u64, u64, u64)> = Vec::new();
    let mut pos = 0u64;
    for (out, l) in outs.iter().zip(&lens) {
        if let Out::Copy(at, len) = out {
            spans.push((*at, *len, pos));
        }
        pos += l;
    }
    spans.sort();
    let map = move |x: u64| -> Option<u64> {
        let i = spans.partition_point(|s| s.0 <= x);
        let s = spans.get(i.checked_sub(1)?)?;
        (x < s.0 + s.1).then(|| s.2 + (x - s.0))
    };
    let mut plan = Plan::default();
    for (out, l) in outs.into_iter().zip(lens) {
        match out {
            Out::Copy(at, len) => plan.copy(at, len),
            Out::Bytes(v) => plan.bytes(v),
            Out::Moov(mut v) => {
                patch_moov(&mut v, &map)?;
                plan.bytes(v)
            }
            Out::Mfra(mut v) => {
                patch_mfra(&mut v, &map)?;
                plan.bytes(v)
            }
            Out::Meta(i) => {
                let (h, rm, ic) = &heifs[i];
                let v = h.build(rm, ic, &map)?;
                if v.len() as u64 != l {
                    return Err(malformed("item locations changed size"));
                }
                plan.bytes(v)
            }
        }
    }
    Ok(plan)
}

fn patch_moov(v: &mut [u8], map: &dyn Fn(u64) -> Option<u64>) -> Result<()> {
    let moov = boxes(&Mem(v), 0, v.len() as u64)?[0];
    for t in kids(v, &moov)?.into_iter().filter(|c| c.is(b"trak")) {
        let Some(stbl) = path(v, &t, &[b"mdia", b"minf", b"stbl"])? else {
            continue;
        };
        for c in kids(v, &stbl)? {
            let s = c.body() as usize;
            let w = match &c.typ {
                b"stco" => 4,
                b"co64" => 8,
                _ => continue,
            };
            let n = be32(v, s + 4)? as usize;
            for k in 0..n {
                let i = s + 8 + w * k;
                let old = uint(v, i, w)?;
                let new =
                    map(old).ok_or_else(|| malformed("a chunk offset points into removed data"))?;
                let mut tmp = Vec::new();
                put_uint(&mut tmp, new, w)?;
                v[i..i + w].copy_from_slice(&tmp);
            }
        }
    }
    Ok(())
}

fn patch_mfra(v: &mut [u8], map: &dyn Fn(u64) -> Option<u64>) -> Result<()> {
    let mfra = boxes(&Mem(v), 0, v.len() as u64)?[0];
    for t in kids(v, &mfra)?.into_iter().filter(|c| c.is(b"tfra")) {
        let s = t.body() as usize;
        let w = if v[s] == 1 { 8 } else { 4 };
        let sizes = be32(v, s + 8)?;
        let extra = ((sizes >> 4 & 3) + 1 + (sizes >> 2 & 3) + 1 + (sizes & 3) + 1) as usize;
        let n = be32(v, s + 12)? as usize;
        let mut i = s + 16;
        for _ in 0..n {
            let at = i + w;
            let old = uint(v, at, w)?;
            if let Some(new) = map(old) {
                let mut tmp = Vec::new();
                put_uint(&mut tmp, new, w)?;
                v[at..at + w].copy_from_slice(&tmp);
            }
            i += 2 * w + extra;
        }
    }
    Ok(())
}
