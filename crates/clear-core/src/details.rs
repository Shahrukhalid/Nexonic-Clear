//! Readable fields from XMP packets, IPTC-IIM records, PNG text chunks,
//! comments and ICC profile headers. Bounded and lossy by design: this is
//! for showing people what a file contains, not for round-tripping.

use crate::exif::Field;

const MAX_FIELDS: usize = 400;
const MAX_VALUE: usize = 300;

fn field(group: &str, name: &str, value: &str) -> Field {
    Field {
        group: group.into(),
        name: name.chars().take(80).collect(),
        value: value.trim().chars().take(MAX_VALUE).collect(),
        tag: None,
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn is_name(s: &str) -> bool {
    let mut parts = s.splitn(2, ':');
    let (Some(p), Some(n)) = (parts.next(), parts.next()) else {
        return false;
    };
    !p.is_empty()
        && !n.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-' | b'.'))
}

/// Properties from an XMP packet: attributes such as `xmp:CreatorTool="…"`
/// and simple elements such as `<dc:title>…</dc:title>` (list items are
/// joined). Structural RDF and namespace declarations are skipped.
pub fn xmp(data: &[u8]) -> Vec<Field> {
    let text = String::from_utf8_lossy(data);
    let mut out: Vec<Field> = Vec::new();
    let skip = |n: &str| {
        n.starts_with("xmlns")
            || n.starts_with("rdf:")
            || n.starts_with("x:")
            || n.starts_with("xml:")
    };
    // Attributes.
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() && out.len() < MAX_FIELDS {
        if bytes[i] == b'=' && bytes.get(i + 1) == Some(&b'"') {
            let start = text[..i]
                .rfind(|c: char| c.is_whitespace() || c == '<')
                .map_or(0, |p| p + 1);
            let name = &text[start..i];
            if let Some(end) = text[i + 2..].find('"') {
                let value = &text[i + 2..i + 2 + end];
                if is_name(name) && !skip(name) && !value.is_empty() {
                    out.push(field("XMP", name, &unescape(value)));
                }
                i += 2 + end;
            }
        }
        i += 1;
    }
    // Elements with text content (innermost only).
    let mut rest: &str = &text;
    while let Some(open) = rest.find('<') {
        if out.len() >= MAX_FIELDS {
            break;
        }
        rest = &rest[open + 1..];
        let Some(close) = rest.find('>') else { break };
        let tag = rest[..close].split_whitespace().next().unwrap_or("");
        if !is_name(tag) || skip(tag) || rest[..close].ends_with('/') {
            continue;
        }
        let end_tag = format!("</{tag}>");
        let Some(end) = rest.find(&end_tag) else {
            continue;
        };
        let inner = &rest[close + 1..end];
        let values: Vec<String> = if inner.contains("<rdf:li") {
            inner
                .split("<rdf:li")
                .skip(1)
                .filter_map(|li| {
                    li.split_once('>')
                        .map(|(_, v)| v.split("</rdf:li>").next().unwrap_or(""))
                })
                .filter(|v| !v.contains('<'))
                .map(unescape)
                .collect()
        } else if !inner.contains('<') {
            vec![unescape(inner)]
        } else {
            continue;
        };
        let joined = values
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        if !joined.is_empty() && !out.iter().any(|f| f.name == tag && f.value == joined) {
            out.push(field("XMP", tag, &joined));
        }
    }
    out
}

fn iptc_name(dataset: u8) -> Option<&'static str> {
    Some(match dataset {
        5 => "Title",
        15 => "Category",
        25 => "Keywords",
        40 => "Special instructions",
        55 => "Date created",
        60 => "Time created",
        62 => "Digital creation date",
        63 => "Digital creation time",
        80 => "Creator",
        85 => "Creator title",
        90 => "City",
        92 => "Sub-location",
        95 => "State or province",
        100 => "Country code",
        101 => "Country",
        103 => "Job identifier",
        105 => "Headline",
        110 => "Credit",
        115 => "Source",
        116 => "Copyright",
        118 => "Contact",
        120 => "Description",
        122 => "Description writer",
        _ => return None,
    })
}

/// IPTC-IIM application records from a Photoshop APP13 payload
/// ("Photoshop 3.0\0" followed by 8BIM resources; IPTC is resource 0x0404).
pub fn iptc(app13: &[u8]) -> Vec<Field> {
    let mut out: Vec<Field> = Vec::new();
    let Some(mut d) = app13.strip_prefix(b"Photoshop 3.0\0") else {
        return out;
    };
    while d.len() >= 12 && d.starts_with(b"8BIM") {
        let id = u16::from_be_bytes([d[4], d[5]]);
        let name_len = d[6] as usize;
        let mut p = 7 + name_len;
        if p % 2 == 1 {
            p += 1;
        }
        let Some(size) = d
            .get(p..p + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
        else {
            break;
        };
        let Some(body) = d.get(p + 4..p + 4 + size) else {
            break;
        };
        if id == 0x0404 {
            let mut i = 0;
            while i + 5 <= body.len() && body[i] == 0x1C && out.len() < MAX_FIELDS {
                let (record, dataset) = (body[i + 1], body[i + 2]);
                let len = u16::from_be_bytes([body[i + 3], body[i + 4]]) as usize;
                if len & 0x8000 != 0 {
                    break; // extended length datasets are not used for text
                }
                let Some(value) = body.get(i + 5..i + 5 + len) else {
                    break;
                };
                if record == 2 && dataset != 0 {
                    let name = iptc_name(dataset)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("Dataset 2:{dataset}"));
                    let v = String::from_utf8_lossy(value);
                    if let Some(f) = out.iter_mut().find(|f| f.name == name && dataset == 25) {
                        f.value = format!("{}; {v}", f.value)
                            .chars()
                            .take(MAX_VALUE)
                            .collect();
                    } else {
                        out.push(field("IPTC", &name, &v));
                    }
                }
                i += 5 + len;
            }
        }
        let next = p + 4 + size + (size & 1);
        if next > d.len() {
            break;
        }
        d = &d[next..];
    }
    out
}

/// A PNG tEXt or uncompressed iTXt chunk as a field; compressed text is noted, not inflated.
pub fn png_text(ty: &[u8; 4], data: &[u8]) -> Option<Field> {
    let nul = data.iter().position(|&b| b == 0)?;
    let key = String::from_utf8_lossy(&data[..nul]).to_string();
    let rest = &data[nul + 1..];
    let value = match ty {
        b"tEXt" => rest.iter().map(|&b| b as char).collect::<String>(),
        b"zTXt" => "(compressed text)".into(),
        b"iTXt" => {
            let compressed = *rest.first()? == 1;
            let after = rest.get(2..)?;
            let lang_end = after.iter().position(|&b| b == 0)?;
            let after = &after[lang_end + 1..];
            let tk_end = after.iter().position(|&b| b == 0)?;
            if compressed {
                "(compressed text)".into()
            } else {
                String::from_utf8_lossy(&after[tk_end + 1..]).to_string()
            }
        }
        _ => return None,
    };
    if key.eq_ignore_ascii_case("XML:com.adobe.xmp") {
        return None; // shown through the XMP reader
    }
    Some(field("Text", &key, &value))
}

pub fn comment(data: &[u8]) -> Field {
    field("Comment", "Comment", &String::from_utf8_lossy(data))
}

/// Colour space, profile class and version from an ICC profile header.
pub fn icc_header(profile: &[u8]) -> Vec<Field> {
    let mut out = Vec::new();
    let text = |r: std::ops::Range<usize>| {
        profile
            .get(r)
            .map(|b| String::from_utf8_lossy(b).trim().to_string())
    };
    if let Some(cs) = text(16..20) {
        out.push(field("ICC", "Colour space", &cs));
    }
    if let Some(class) = text(12..16) {
        out.push(field("ICC", "Profile class", &class));
    }
    if let (Some(&maj), Some(&min)) = (profile.get(8), profile.get(9)) {
        out.push(field("ICC", "Version", &format!("{maj}.{}", min >> 4)));
    }
    out
}
