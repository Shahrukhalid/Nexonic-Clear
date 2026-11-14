//! PDF: document information, XMP metadata streams, application private data,
//! embedded Content Credentials and attachments.
//!
//! Cleaning always writes a complete new file from the latest revision and
//! drops every unreferenced object, so nothing removed survives in an older
//! revision. The fingerprint hashes the page tree (content streams, resources,
//! annotations) with references resolved, so it is independent of object
//! numbers and proves the visible document is unchanged.

use crate::{details, malformed, Block, CleanOptions, Digest, Field, Kind, Parsed, Result};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::HashSet;

const SKIP: &[&[u8]] = &[b"Metadata", b"PieceInfo", b"LastModified", b"Parent", b"AF"];

fn load(data: &[u8]) -> Result<Document> {
    let doc = Document::load_mem(data).map_err(|e| crate::Error::Malformed(format!("PDF: {e}")))?;
    if doc.is_encrypted() || doc.trailer.has(b"Encrypt") {
        return Err(malformed("encrypted PDFs are not supported"));
    }
    Ok(doc)
}

fn resolve<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
    match o {
        Object::Reference(id) => doc.get_object(*id).ok(),
        o => Some(o),
    }
}

fn dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    match resolve(doc, o)? {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn name_is(d: &Dictionary, key: &[u8], v: &[u8]) -> bool {
    matches!(d.get(key), Ok(Object::Name(n)) if n == v)
}

fn text(o: &Object) -> String {
    match o {
        Object::String(b, _) => {
            if b.starts_with(&[0xFE, 0xFF]) {
                let u: Vec<u16> = b[2..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_be_bytes(*c))
                    .collect();
                String::from_utf16_lossy(&u)
            } else if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
                String::from_utf8_lossy(&b[3..]).into()
            } else {
                b.iter().map(|&c| c as char).collect()
            }
        }
        Object::Name(n) => String::from_utf8_lossy(n).into(),
        Object::Integer(i) => i.to_string(),
        Object::Real(r) => r.to_string(),
        Object::Boolean(b) => b.to_string(),
        _ => String::new(),
    }
}

/// A file specification that holds Content Credentials.
fn is_c2pa_spec(doc: &Document, spec: &Object) -> bool {
    let Some(d) = dict(doc, spec) else {
        return false;
    };
    if name_is(d, b"AFRelationship", b"C2PA_Manifest") {
        return true;
    }
    let ef = d.get(b"EF").ok().and_then(|o| dict(doc, o));
    let f = ef.and_then(|e| e.get(b"F").ok()).and_then(|o| dict(doc, o));
    f.is_some_and(|s| {
        matches!(s.get(b"Subtype"), Ok(Object::Name(n)) if n.eq_ignore_ascii_case(b"application/c2pa") || n.eq_ignore_ascii_case(b"application#2Fc2pa"))
    })
}

/// (name, file spec) pairs from the EmbeddedFiles name tree.
fn embedded_files(doc: &Document) -> Vec<(String, Object)> {
    let mut out = Vec::new();
    let Some(names) = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Names").ok())
        .and_then(|o| dict(doc, o))
    else {
        return out;
    };
    let Some(root) = names.get(b"EmbeddedFiles").ok().and_then(|o| dict(doc, o)) else {
        return out;
    };
    let mut stack = vec![root];
    let mut guard = 0;
    while let Some(node) = stack.pop() {
        guard += 1;
        if guard > 10_000 {
            break;
        }
        if let Ok(Object::Array(a)) = node.get(b"Names").map(|o| resolve(doc, o).unwrap_or(o)) {
            for pair in a.chunks(2) {
                if let [k, v] = pair {
                    out.push((text(resolve(doc, k).unwrap_or(k)), v.clone()));
                }
            }
        }
        if let Ok(Object::Array(kids)) = node.get(b"Kids").map(|o| resolve(doc, o).unwrap_or(o)) {
            stack.extend(kids.iter().filter_map(|k| dict(doc, k)));
        }
    }
    out
}

struct Hasher<'a> {
    doc: &'a Document,
    seen: std::collections::HashMap<ObjectId, usize>,
    d: Digest,
}

impl Hasher<'_> {
    fn obj(&mut self, o: &Object, depth: usize) {
        if depth > 64 {
            self.d.add(b"deep");
            return;
        }
        match o {
            Object::Reference(id) => {
                if let Some(n) = self.seen.get(id) {
                    self.d.add(b"R");
                    self.d.add(&n.to_le_bytes());
                    return;
                }
                let n = self.seen.len();
                self.seen.insert(*id, n);
                match self.doc.get_object(*id) {
                    Ok(t) => self.obj(t, depth + 1),
                    Err(_) => self.d.add(b"missing"),
                }
            }
            Object::Dictionary(d) => self.dict(d, depth),
            Object::Stream(s) => {
                self.d.add(b"S");
                self.dict(&s.dict, depth);
                self.d.add(&s.content);
            }
            Object::Array(a) => {
                self.d.add(b"A");
                for x in a {
                    self.obj(x, depth + 1);
                }
                self.d.add(b"a");
            }
            Object::Null => self.d.add(b"N"),
            Object::Boolean(b) => self.d.add(if *b { b"T" } else { b"F" }),
            Object::Integer(i) => self.d.add(&i.to_le_bytes()),
            Object::Real(r) => self.d.add(&r.to_le_bytes()),
            Object::Name(n) => {
                self.d.add(b"/");
                self.d.add(n);
            }
            Object::String(s, _) => {
                self.d.add(b"(");
                self.d.add(s);
            }
        }
    }
    fn dict(&mut self, d: &Dictionary, depth: usize) {
        let mut keys: Vec<(&Vec<u8>, &Object)> = d
            .iter()
            .filter(|(k, _)| !SKIP.contains(&k.as_slice()) && k.as_slice() != b"Length")
            .collect();
        keys.sort_by(|a, b| a.0.cmp(b.0));
        self.d.add(b"D");
        for (k, v) in keys {
            self.d.add(k);
            self.obj(v, depth + 1);
        }
        self.d.add(b"d");
    }
}

fn media_box(doc: &Document, page: ObjectId) -> (u32, u32) {
    let mut cur = doc.get_object(page).ok().and_then(|o| dict(doc, o));
    for _ in 0..32 {
        let Some(d) = cur else { break };
        if let Ok(Object::Array(a)) = d.get(b"MediaBox").map(|o| resolve(doc, o).unwrap_or(o)) {
            let n: Vec<f64> = a
                .iter()
                .filter_map(|x| match resolve(doc, x)? {
                    Object::Integer(i) => Some(*i as f64),
                    Object::Real(r) => Some(*r as f64),
                    _ => None,
                })
                .collect();
            if n.len() == 4 {
                return (
                    (n[2] - n[0]).abs().round() as u32,
                    (n[3] - n[1]).abs().round() as u32,
                );
            }
        }
        cur = d.get(b"Parent").ok().and_then(|o| dict(doc, o));
    }
    (0, 0)
}

fn f(group: &str, name: &str, value: String) -> Field {
    Field {
        group: group.into(),
        name: name.into(),
        value,
        tag: None,
    }
}

pub(crate) fn parse(data: &[u8]) -> Result<Parsed> {
    let doc = load(data)?;
    let mut p = Parsed::empty();
    let pages = doc.get_pages();
    if pages.is_empty() {
        return Err(malformed("PDF has no pages"));
    }
    if let Some(first) = pages.values().next() {
        (p.width, p.height) = media_box(&doc, *first);
    }
    p.fields.push(f("PDF", "Pages", pages.len().to_string()));
    // Document information dictionary.
    if let Some(info) = doc.trailer.get(b"Info").ok().and_then(|o| dict(&doc, o)) {
        let mut size = 0;
        for (k, v) in info.iter() {
            let v = text(resolve(&doc, v).unwrap_or(v));
            size += k.len() + v.len();
            p.fields.push(f("PDF", &String::from_utf8_lossy(k), v));
        }
        p.blocks.push(Block {
            kind: Kind::Comment,
            label: "Document information (title, author, dates, software)".into(),
            size,
        });
    }
    // Metadata streams, private application data and C2PA everywhere.
    let mut xmp = 0;
    let mut piece = 0;
    for o in doc.objects.values() {
        let d = match o {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &s.dict,
            _ => continue,
        };
        if let Ok(m) = d.get(b"Metadata") {
            if let Some(Object::Stream(s)) = resolve(&doc, m) {
                xmp += 1;
                let content = s
                    .decompressed_content()
                    .unwrap_or_else(|_| s.content.clone());
                if xmp == 1 {
                    p.fields.extend(details::xmp(&content));
                }
                p.blocks.push(Block {
                    kind: Kind::Xmp,
                    label: if xmp == 1 {
                        "XMP metadata stream".into()
                    } else {
                        "XMP metadata stream (page or image)".into()
                    },
                    size: s.content.len(),
                });
            }
        }
        if d.has(b"PieceInfo") || d.has(b"LastModified") {
            piece += 1;
        }
    }
    if piece > 0 {
        p.blocks.push(Block {
            kind: Kind::Other,
            label: format!("Application private data ({piece})"),
            size: 0,
        });
    }
    let mut emb = Digest::new();
    let mut any = false;
    let mut c2pa_specs = Vec::new();
    let mut files = embedded_files(&doc);
    if let Ok(Object::Array(af)) = doc
        .catalog()
        .and_then(|c| c.get(b"AF"))
        .map(|o| resolve(&doc, o).unwrap_or(o))
    {
        c2pa_specs.extend(af.iter().filter(|s| is_c2pa_spec(&doc, s)).cloned());
        for spec in af.iter().filter(|s| !is_c2pa_spec(&doc, s)) {
            if !files.iter().any(|f| &f.1 == spec) {
                let name = dict(&doc, spec)
                    .and_then(|d| d.get(b"UF").or(d.get(b"F")).ok())
                    .map(text)
                    .unwrap_or_default();
                files.push((name, spec.clone()));
            }
        }
    }
    for (name, spec) in files {
        if is_c2pa_spec(&doc, &spec) {
            if !c2pa_specs.contains(&spec) {
                c2pa_specs.push(spec);
            }
            continue;
        }
        let mut h = Hasher {
            doc: &doc,
            seen: Default::default(),
            d: Digest::new(),
        };
        h.obj(&spec, 0);
        emb.add(name.as_bytes());
        emb.add(&h.d.finish());
        any = true;
        p.blocks.push(Block {
            kind: Kind::Embedded,
            label: format!("Attached file: {name}"),
            size: 0,
        });
    }
    for _ in &c2pa_specs {
        p.blocks.push(Block {
            kind: Kind::C2pa,
            label: "C2PA manifest (embedded file)".into(),
            size: 0,
        });
    }
    p.embedded = any.then(|| emb.finish());
    let mut h = Hasher {
        doc: &doc,
        seen: Default::default(),
        d: Digest::new(),
    };
    for id in pages.values() {
        h.obj(&Object::Reference(*id), 0);
    }
    if let Ok(af) = doc.catalog().and_then(|c| c.get(b"AcroForm")) {
        h.obj(af, 0);
    }
    p.fingerprint = h.d.finish();
    Ok(p)
}

pub(crate) fn clean(data: &[u8], o: &CleanOptions) -> Result<Vec<u8>> {
    // Rewriting normalises the file; repeat until the output is stable so that
    // cleaning a cleaned file gives the same bytes.
    let mut out = clean_once(data, o)?;
    for _ in 0..3 {
        let again = clean_once(&out, o)?;
        if again == out {
            return Ok(out);
        }
        out = again;
    }
    Err(malformed("the PDF could not be rewritten consistently"))
}

fn clean_once(data: &[u8], o: &CleanOptions) -> Result<Vec<u8>> {
    let mut doc = load(data)?;
    if o.comments {
        doc.trailer.remove(b"Info");
    }
    // Collect C2PA file specifications before editing.
    let mut c2pa: HashSet<ObjectId> = HashSet::new();
    let mut c2pa_direct = Vec::new();
    if o.c2pa {
        let mut specs: Vec<Object> = embedded_files(&doc)
            .into_iter()
            .map(|e| e.1)
            .filter(|s| is_c2pa_spec(&doc, s))
            .collect();
        if let Ok(Object::Array(af)) = doc
            .catalog()
            .and_then(|c| c.get(b"AF"))
            .map(|x| resolve(&doc, x).unwrap_or(x))
        {
            specs.extend(af.iter().filter(|s| is_c2pa_spec(&doc, s)).cloned());
        }
        for s in specs {
            match s {
                Object::Reference(id) => {
                    c2pa.insert(id);
                }
                other => c2pa_direct.push(other),
            }
        }
    }
    let is_c2pa = |x: &Object| match x {
        Object::Reference(id) => c2pa.contains(id),
        other => c2pa_direct.contains(other),
    };
    // Edit every dictionary in place.
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        let Some(obj) = doc.objects.get_mut(&id) else {
            continue;
        };
        let d = match obj {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &mut s.dict,
            _ => continue,
        };
        strip(d, o, &is_c2pa);
    }
    if let Ok(cat) = doc.catalog_mut() {
        strip(cat, o, &is_c2pa);
    }
    // Remove attachments and C2PA entries from the EmbeddedFiles name tree.
    let names_id = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Names").ok())
        .cloned();
    if let Some(n) = names_id {
        let ef = dict(&doc, &n)
            .and_then(|d| d.get(b"EmbeddedFiles").ok())
            .cloned();
        if let Some(ef) = ef {
            let mut nodes = vec![ef];
            let mut guard = 0;
            while let Some(node) = nodes.pop() {
                guard += 1;
                if guard > 10_000 {
                    break;
                }
                let kids: Vec<Object> = match dict(&doc, &node)
                    .and_then(|d| d.get(b"Kids").ok())
                    .and_then(|k| resolve(&doc, k))
                {
                    Some(Object::Array(a)) => a.clone(),
                    _ => Vec::new(),
                };
                nodes.extend(kids);
                let target: Option<&mut Dictionary> = match &node {
                    Object::Reference(id) => match doc.objects.get_mut(id) {
                        Some(Object::Dictionary(d)) => Some(d),
                        _ => None,
                    },
                    _ => None,
                };
                let Some(d) = target else { continue };
                if let Ok(Object::Array(a)) = d.get_mut(b"Names") {
                    let mut kept = Vec::new();
                    for pair in a.chunks(2) {
                        if let [k, v] = pair {
                            if o.embedded && !is_c2pa(v) || o.c2pa && is_c2pa(v) {
                                continue;
                            }
                            kept.push(k.clone());
                            kept.push(v.clone());
                        }
                    }
                    *a = kept;
                }
            }
        }
        if o.embedded && o.c2pa {
            if let Some(d) = match &n {
                Object::Reference(id) => match doc.objects.get_mut(id) {
                    Some(Object::Dictionary(d)) => Some(d),
                    _ => None,
                },
                _ => None,
            } {
                d.remove(b"EmbeddedFiles");
            } else if let Ok(cat) = doc.catalog_mut() {
                if let Ok(Object::Dictionary(d)) = cat.get_mut(b"Names") {
                    d.remove(b"EmbeddedFiles");
                }
            }
        }
    }
    doc.prune_objects();
    let mut out = Vec::with_capacity(data.len());
    doc.save_to(&mut out)
        .map_err(|e| crate::Error::Malformed(format!("PDF write failed: {e}")))?;
    Ok(out)
}

fn strip(d: &mut Dictionary, o: &CleanOptions, is_c2pa: &dyn Fn(&Object) -> bool) {
    if o.xmp {
        d.remove(b"Metadata");
    }
    if o.other {
        d.remove(b"PieceInfo");
        d.remove(b"LastModified");
    }
    if o.c2pa || o.embedded {
        if let Ok(Object::Array(af)) = d.get_mut(b"AF") {
            af.retain(|x| if is_c2pa(x) { !o.c2pa } else { !o.embedded });
            if af.is_empty() {
                d.remove(b"AF");
            }
        }
    }
}
