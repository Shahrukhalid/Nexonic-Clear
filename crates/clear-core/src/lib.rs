//! Nexonic Clear core: inspect and selectively remove image metadata and
//! embedded Content Credentials (C2PA) without re-encoding the image.
//!
//! Every format is handled at the container level: metadata segments/chunks
//! are dropped or kept, and the compressed image data is copied byte for byte.
//! [`image_fingerprint`] hashes only the image-data parts so callers can prove
//! that cleaning did not touch the picture.

#[cfg(feature = "c2pa")]
mod credentials;
mod exif;
mod jpeg;
mod png;
mod webp;

use serde::{Deserialize, Serialize};

pub use exif::ExifSummary;

/// Largest file accepted, to bound memory use on phones.
pub const MAX_FILE_SIZE: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Jpeg,
    Png,
    Webp,
}

impl Format {
    pub fn detect(data: &[u8]) -> Option<Self> {
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Format::Jpeg)
        } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Format::Png)
        } else if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
            Some(Format::Webp)
        } else {
            None
        }
    }
    pub fn mime(self) -> &'static str {
        match self {
            Format::Jpeg => "image/jpeg",
            Format::Png => "image/png",
            Format::Webp => "image/webp",
        }
    }
}

/// Kinds of metadata a user can keep or remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Exif,
    Xmp,
    Iptc,
    Icc,
    C2pa,
    Comment,
    Other,
}

/// One metadata block found in the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub kind: Kind,
    /// Container label, e.g. "APP1 Exif", "iTXt XML:com.adobe.xmp", "EXIF chunk".
    pub label: String,
    pub size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Credentials {
    pub present: bool,
    /// "valid", "trusted", "invalid", or "unreadable".
    pub state: String,
    pub claim_generator: Option<String>,
    pub title: Option<String>,
    pub issuer: Option<String>,
    pub signed_at: Option<String>,
    pub algorithm: Option<String>,
    pub ai_generated: bool,
    pub actions: Vec<String>,
    pub manifests: usize,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inspection {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub animated: bool,
    pub size: usize,
    pub blocks: Vec<Block>,
    pub exif: Option<ExifSummary>,
    pub credentials: Credentials,
    pub image_fingerprint: String,
}

impl Inspection {
    pub fn has(&self, kind: Kind) -> bool {
        self.blocks.iter().any(|b| b.kind == kind)
    }
}

/// What to remove. Anything not selected is kept unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanOptions {
    pub exif: bool,
    pub xmp: bool,
    pub iptc: bool,
    pub icc: bool,
    pub c2pa: bool,
    pub comments: bool,
    pub other: bool,
    /// When EXIF is removed, write back only the orientation tag so the image
    /// still displays the right way up.
    pub keep_orientation: bool,
}

impl Default for CleanOptions {
    fn default() -> Self {
        Self {
            exif: true,
            xmp: true,
            iptc: true,
            icc: false,
            c2pa: false,
            comments: true,
            other: true,
            keep_orientation: true,
        }
    }
}

impl CleanOptions {
    pub fn removes(&self, kind: Kind) -> bool {
        match kind {
            Kind::Exif => self.exif,
            Kind::Xmp => self.xmp,
            Kind::Iptc => self.iptc,
            Kind::Icc => self.icc,
            Kind::C2pa => self.c2pa,
            Kind::Comment => self.comments,
            Kind::Other => self.other,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanReport {
    pub before: Inspection,
    pub after: Inspection,
    /// True when the compressed image data is byte-for-byte identical.
    pub image_unchanged: bool,
    /// Kinds that were requested for removal but are still present (should be empty).
    pub still_present: Vec<Kind>,
    pub bytes_removed: i64,
    /// Plain-language notes, e.g. that keeping credentials while editing other
    /// metadata breaks their hash binding.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Error {
    Unsupported,
    TooLarge,
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unsupported => write!(
                f,
                "Unsupported file. Nexonic Clear handles JPEG, PNG and WebP."
            ),
            Error::TooLarge => write!(f, "The file is larger than 256 MB."),
            Error::Malformed(m) => write!(f, "The image file is damaged: {m}"),
        }
    }
}
impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) struct Parsed {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub animated: bool,
    pub blocks: Vec<Block>,
    pub exif_tiff: Option<Vec<u8>>,
    pub fingerprint: u64,
}

fn parse(format: Format, data: &[u8]) -> Result<Parsed> {
    match format {
        Format::Jpeg => jpeg::parse(data),
        Format::Png => png::parse(data),
        Format::Webp => webp::parse(data),
    }
}

fn check(data: &[u8]) -> Result<Format> {
    if data.len() > MAX_FILE_SIZE {
        return Err(Error::TooLarge);
    }
    Format::detect(data).ok_or(Error::Unsupported)
}

/// Inspect an image held in memory.
pub fn inspect(data: &[u8]) -> Result<Inspection> {
    let format = check(data)?;
    let p = parse(format, data)?;
    let exif = p.exif_tiff.as_deref().and_then(exif::summarize);
    #[cfg(feature = "c2pa")]
    let credentials = if p.blocks.iter().any(|b| b.kind == Kind::C2pa) {
        credentials::read(format, data)
    } else {
        Credentials::default()
    };
    #[cfg(not(feature = "c2pa"))]
    let credentials = Credentials {
        present: p.blocks.iter().any(|b| b.kind == Kind::C2pa),
        state: "unreadable".into(),
        ..Default::default()
    };
    Ok(Inspection {
        format,
        width: p.width,
        height: p.height,
        has_alpha: p.has_alpha,
        animated: p.animated,
        size: data.len(),
        blocks: p.blocks,
        exif,
        credentials,
        image_fingerprint: format!("{:016x}", p.fingerprint),
    })
}

/// Remove the selected metadata and return the new file bytes.
pub fn clean(data: &[u8], opts: &CleanOptions) -> Result<Vec<u8>> {
    let format = check(data)?;
    let p = parse(format, data)?;
    let orientation = if opts.exif && opts.keep_orientation {
        p.exif_tiff
            .as_deref()
            .and_then(exif::orientation)
            .filter(|&o| o != 1)
    } else {
        None
    };
    let minimal_exif = orientation.map(exif::minimal_with_orientation);
    match format {
        Format::Jpeg => jpeg::clean(data, opts, minimal_exif.as_deref()),
        Format::Png => png::clean(data, opts, minimal_exif.as_deref()),
        Format::Webp => webp::clean(data, opts, minimal_exif.as_deref()),
    }
}

/// Clean and verify the result by inspecting it again.
pub fn clean_and_verify(data: &[u8], opts: &CleanOptions) -> Result<(Vec<u8>, CleanReport)> {
    let before = inspect(data)?;
    let out = clean(data, opts)?;
    let after = inspect(&out)?;
    let mut still_present: Vec<Kind> = Vec::new();
    for b in &after.blocks {
        let minimal_orientation = b.kind == Kind::Exif && opts.keep_orientation;
        if opts.removes(b.kind) && !minimal_orientation && !still_present.contains(&b.kind) {
            still_present.push(b.kind);
        }
    }
    if opts.exif && opts.keep_orientation {
        if let Some(e) = &after.exif {
            if e.fields.iter().any(|(k, _)| k != "Orientation")
                && !still_present.contains(&Kind::Exif)
            {
                still_present.push(Kind::Exif);
            }
        }
    }
    let mut notes = Vec::new();
    if before.credentials.present && !opts.c2pa {
        let other_removed = before
            .blocks
            .iter()
            .any(|b| b.kind != Kind::C2pa && opts.removes(b.kind));
        if other_removed && after.credentials.state == "invalid" {
            notes.push("Content Credentials were kept, but they no longer match the file because other metadata changed. Viewers will report them as invalid.".into());
        }
    }
    if before.credentials.present && opts.c2pa {
        notes.push("Embedded Content Credentials were removed from this file. Copies elsewhere, cloud records and platform labels are not affected.".into());
    }
    let image_unchanged = before.image_fingerprint == after.image_fingerprint;
    let bytes_removed = before.size as i64 - after.size as i64;
    Ok((
        out,
        CleanReport {
            before,
            after,
            image_unchanged,
            still_present,
            bytes_removed,
            notes,
        },
    ))
}

/// FNV-1a 64-bit, used only to compare image data before and after cleaning.
pub(crate) struct Fnv(u64);
impl Fnv {
    pub fn new() -> Self {
        Fnv(0xcbf29ce484222325)
    }
    pub fn add(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    pub fn finish(&self) -> u64 {
        self.0
    }
}

pub(crate) fn be16(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(i..i + 2)?.try_into().ok()?))
}
pub(crate) fn be32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?))
}
pub(crate) fn le32(d: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(i..i + 4)?.try_into().ok()?))
}
pub(crate) fn le24(d: &[u8], i: usize) -> Option<u32> {
    let s = d.get(i..i + 3)?;
    Some(s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16)
}
pub(crate) fn malformed(m: &str) -> Error {
    Error::Malformed(m.into())
}
