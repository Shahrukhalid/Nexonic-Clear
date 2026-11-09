//! Nexonic Clear core: inspect and selectively remove metadata and embedded
//! Content Credentials (C2PA) from images, audio, video and PDF documents
//! without re-encoding anything.
//!
//! Every format is handled at the container level: metadata segments/chunks
//! are dropped or kept, and the compressed image data is copied byte for byte.
//! `image_fingerprint` is a SHA-256 over the image-data parts only, so callers
//! can prove that cleaning did not touch the picture. Embedded motion-photo
//! videos and extra images are tracked separately and kept unless removed.

mod bmff;
#[cfg(feature = "c2pa")]
pub mod credentials;
mod details;
mod exif;
mod jpeg;
mod mp3;
mod pdf;
mod png;
mod stream;
mod wav;
mod webp;

use stream::{FileSrc, Mem, Src};

use serde::{Deserialize, Serialize};

pub use exif::{ExifSummary, Field};

/// A plain-language message with a stable code, so apps can translate it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub code: String,
    pub text: String,
}

fn msg(code: &str, text: &str) -> Message {
    Message {
        code: code.into(),
        text: text.into(),
    }
}

/// Largest file loaded into memory, to bound memory use on phones.
pub const MAX_FILE_SIZE: usize = 256 * 1024 * 1024;

/// Largest audio or video file processed from disk by streaming
/// ([`inspect_path`], [`clean_path`]).
pub const MAX_STREAM_SIZE: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Jpeg,
    Png,
    Webp,
    Heif,
    Avif,
    Wav,
    Mp3,
    Mp4,
    Mov,
    M4a,
    Pdf,
}

impl Format {
    pub fn detect(data: &[u8]) -> Option<Self> {
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(Format::Jpeg)
        } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Format::Png)
        } else if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
            Some(Format::Webp)
        } else if data.len() >= 12
            && matches!(&data[0..4], b"RIFF" | b"RF64" | b"BW64")
            && &data[8..12] == b"WAVE"
        {
            Some(Format::Wav)
        } else if data.len() >= 12 && &data[4..8] == b"ftyp" {
            Self::brand(data)
        } else if data.starts_with(b"%PDF-") {
            Some(Format::Pdf)
        } else if data.starts_with(b"ID3")
            || (data.len() >= 4 && data[0] == 0xFF && data[1] & 0xE0 == 0xE0 && data[1] & 0x06 != 0)
        {
            Some(Format::Mp3)
        } else {
            None
        }
    }
    fn brand(data: &[u8]) -> Option<Self> {
        let size = (be32(data, 0)? as usize).clamp(8, data.len().min(4096));
        let major = &data[8..12];
        let compat: Vec<&[u8]> = data
            .get(16..size)
            .unwrap_or_default()
            .chunks_exact(4)
            .collect();
        let has = |b: &[u8]| major == b || compat.contains(&b);
        Some(match major {
            b"avif" | b"avis" => Format::Avif,
            b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" => Format::Heif,
            b"mif1" | b"msf1" if has(b"avif") || has(b"avis") => Format::Avif,
            b"mif1" | b"msf1" => Format::Heif,
            b"qt  " => Format::Mov,
            b"M4A " | b"M4B " | b"M4P " => Format::M4a,
            _ => Format::Mp4,
        })
    }
    pub fn mime(self) -> &'static str {
        match self {
            Format::Jpeg => "image/jpeg",
            Format::Png => "image/png",
            Format::Webp => "image/webp",
            Format::Heif => "image/heic",
            Format::Avif => "image/avif",
            Format::Wav => "audio/wav",
            Format::Mp3 => "audio/mpeg",
            Format::Mp4 => "video/mp4",
            Format::Mov => "video/quicktime",
            Format::M4a => "audio/mp4",
            Format::Pdf => "application/pdf",
        }
    }
    /// The usual file extension, without a dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
            Format::Webp => "webp",
            Format::Heif => "heic",
            Format::Avif => "avif",
            Format::Wav => "wav",
            Format::Mp3 => "mp3",
            Format::Mp4 => "mp4",
            Format::Mov => "mov",
            Format::M4a => "m4a",
            Format::Pdf => "pdf",
        }
    }
    /// "image", "audio", "video" or "document".
    pub fn media(self) -> &'static str {
        match self {
            Format::Jpeg | Format::Png | Format::Webp | Format::Heif | Format::Avif => "image",
            Format::Wav | Format::Mp3 | Format::M4a => "audio",
            Format::Mp4 | Format::Mov => "video",
            Format::Pdf => "document",
        }
    }
    /// Formats that can be processed from disk without loading them whole.
    pub fn streams(self) -> bool {
        matches!(
            self,
            Format::Wav | Format::Mp3 | Format::Mp4 | Format::Mov | Format::M4a
        )
    }
    /// File extensions (lower case, without a dot) that may hold a supported file.
    pub const EXTENSIONS: &'static [&'static str] = &[
        "jpg", "jpeg", "png", "webp", "heic", "heif", "avif", "wav", "mp3", "mp4", "m4v", "mov",
        "m4a", "pdf",
    ];
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
    /// Motion-photo video, depth maps, HDR gain maps and other images stored
    /// after the main picture, with their multi-picture index.
    Embedded,
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
    /// "embedded" or "sidecar" (a separate `.c2pa` file); empty when absent.
    #[serde(default)]
    pub source: String,
    /// One result per validation profile: "signature", "official", "interim".
    #[serde(default)]
    pub profiles: Vec<ProfileResult>,
    /// Every manifest in the store, active manifest first.
    #[serde(default)]
    pub store: Vec<ManifestInfo>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProfileResult {
    pub id: String,
    /// "trusted", "valid", "invalid" or "unreadable".
    pub state: String,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ManifestInfo {
    pub label: String,
    pub active: bool,
    pub claim_version: Option<u8>,
    pub title: Option<String>,
    pub format: Option<String>,
    pub generator: Option<String>,
    pub issuer: Option<String>,
    pub signed_at: Option<String>,
    pub algorithm: Option<String>,
    pub assertions: Vec<String>,
    pub ingredients: Vec<IngredientInfo>,
    pub success: Vec<String>,
    pub informational: Vec<String>,
    pub failure: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IngredientInfo {
    pub title: Option<String>,
    pub format: Option<String>,
    pub relationship: String,
    pub manifest: Option<String>,
    pub failure: Vec<String>,
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
    /// SHA-256 of the image data (hex).
    pub image_fingerprint: String,
    /// SHA-256 of embedded motion-photo or extra-image content, if present.
    #[serde(default)]
    pub embedded_fingerprint: Option<String>,
    /// Facts the user should know, e.g. a motion photo whose descriptor is missing.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Every readable field: EXIF, GPS, XMP, IPTC, text chunks, comments, ICC header.
    #[serde(default)]
    pub details: Vec<Field>,
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
    /// Remove motion-photo video and extra embedded images (off by default).
    #[serde(default)]
    pub embedded: bool,
    /// When EXIF is removed, write back only the orientation tag so the image
    /// still displays the right way up.
    pub keep_orientation: bool,
    /// When EXIF is removed, keep the capture and modification dates and time zones.
    #[serde(default)]
    pub keep_dates: bool,
    /// When EXIF is removed, keep the print resolution (DPI).
    #[serde(default = "yes")]
    pub keep_resolution: bool,
}

fn yes() -> bool {
    true
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
            embedded: false,
            keep_orientation: true,
            keep_dates: false,
            keep_resolution: true,
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
            Kind::Embedded => self.embedded,
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
    /// True only when every check passed: same format, dimensions, alpha and
    /// animation; identical image data; kept embedded content identical; and
    /// nothing selected left behind. Unverified results must not be exported.
    #[serde(default)]
    pub verified: bool,
    /// Why verification failed, if it did.
    #[serde(default)]
    pub problems: Vec<Message>,
    /// Plain-language notes, e.g. that keeping credentials while editing other
    /// metadata breaks their hash binding.
    pub notes: Vec<Message>,
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
                "Unsupported file. Nexonic Clear handles JPEG, PNG, WebP, HEIC, AVIF, WAV, MP3, MP4, MOV, M4A and PDF."
            ),
            Error::TooLarge => write!(f, "The file is too large."),
            Error::Malformed(m) => write!(f, "The file is damaged: {m}"),
        }
    }
}
impl std::error::Error for Error {}

impl Error {
    /// Stable code for apps: "unsupported", "too_large" or "malformed".
    pub fn code(&self) -> &'static str {
        match self {
            Error::Unsupported => "unsupported",
            Error::TooLarge => "too_large",
            Error::Malformed(_) => "malformed",
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) struct Parsed {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub animated: bool,
    pub blocks: Vec<Block>,
    pub exif_tiff: Option<Vec<u8>>,
    pub fingerprint: [u8; 32],
    pub embedded: Option<[u8; 32]>,
    pub warnings: Vec<String>,
    pub fields: Vec<Field>,
}

impl Parsed {
    pub fn empty() -> Self {
        Parsed {
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
        }
    }
}

fn parse(format: Format, data: &[u8]) -> Result<Parsed> {
    match format {
        Format::Jpeg => jpeg::parse(data),
        Format::Png => png::parse(data),
        Format::Webp => webp::parse(data),
        Format::Pdf => pdf::parse(data),
        _ => parse_src(format, &Mem(data)),
    }
}

fn parse_src(format: Format, src: &dyn Src) -> Result<Parsed> {
    match format {
        Format::Wav => wav::parse(src),
        Format::Mp3 => mp3::parse(src),
        Format::Mp4 | Format::Mov | Format::M4a | Format::Heif | Format::Avif => bmff::parse(src),
        _ => Err(Error::Unsupported),
    }
}

fn plan(format: Format, src: &dyn Src, opts: &CleanOptions) -> Result<stream::Plan> {
    match format {
        Format::Wav => wav::clean(src, opts),
        Format::Mp3 => mp3::clean(src, opts),
        Format::Mp4 | Format::Mov | Format::M4a | Format::Heif | Format::Avif => {
            bmff::clean(src, opts)
        }
        _ => Err(Error::Unsupported),
    }
}

fn check(data: &[u8]) -> Result<Format> {
    if data.len() > MAX_FILE_SIZE {
        return Err(Error::TooLarge);
    }
    Format::detect(data).ok_or(Error::Unsupported)
}

/// Where the file being read lives, for the credential reader.
#[derive(Clone, Copy)]
pub(crate) enum Input<'a> {
    Bytes(&'a [u8]),
    Path(&'a std::path::Path),
}

/// Inspect a file held in memory.
pub fn inspect(data: &[u8]) -> Result<Inspection> {
    inspect_inner(data, None)
}

/// Inspects a file together with a separate `.c2pa` manifest file. The
/// sidecar is validated against the file; embedded credentials, if any,
/// take precedence.
pub fn inspect_with_sidecar(data: &[u8], sidecar: &[u8]) -> Result<Inspection> {
    if sidecar.len() > MAX_SIDECAR {
        return Err(Error::TooLarge);
    }
    inspect_inner(data, Some(sidecar))
}

/// Largest accepted `.c2pa` sidecar.
pub const MAX_SIDECAR: usize = 32 * 1024 * 1024;

fn inspect_inner(data: &[u8], sidecar: Option<&[u8]>) -> Result<Inspection> {
    let format = check(data)?;
    let p = parse(format, data)?;
    Ok(build(format, p, data.len(), Input::Bytes(data), sidecar))
}

fn build(
    format: Format,
    p: Parsed,
    size: usize,
    input: Input,
    sidecar: Option<&[u8]>,
) -> Inspection {
    #[cfg(not(feature = "c2pa"))]
    let _ = (sidecar, input);
    let exif = p.exif_tiff.as_deref().and_then(exif::summarize);
    let mut details = p
        .exif_tiff
        .as_deref()
        .map(exif::all_fields)
        .unwrap_or_default();
    details.extend(p.fields);
    details.truncate(2000);
    #[cfg(feature = "c2pa")]
    let credentials = if p.blocks.iter().any(|b| b.kind == Kind::C2pa) {
        credentials::read(format, input, credentials::Source::Embedded)
    } else if let Some(m) = sidecar {
        credentials::read(format, input, credentials::Source::Sidecar(m))
    } else {
        Credentials::default()
    };
    #[cfg(not(feature = "c2pa"))]
    let credentials = Credentials {
        present: p.blocks.iter().any(|b| b.kind == Kind::C2pa),
        state: "unreadable".into(),
        ..Default::default()
    };
    Inspection {
        format,
        width: p.width,
        height: p.height,
        has_alpha: p.has_alpha,
        animated: p.animated,
        size,
        blocks: p.blocks,
        exif,
        credentials,
        image_fingerprint: hex(&p.fingerprint),
        embedded_fingerprint: p.embedded.as_ref().map(|d| hex(d)),
        warnings: p.warnings,
        details,
    }
}

/// Remove the selected metadata and return the new file bytes.
pub fn clean(data: &[u8], opts: &CleanOptions) -> Result<Vec<u8>> {
    let format = check(data)?;
    match format {
        Format::Jpeg | Format::Png | Format::Webp => {}
        Format::Pdf => {
            pdf::parse(data)?;
            return pdf::clean(data, opts);
        }
        _ => {
            // Only files that parse completely are rewritten.
            parse_src(format, &Mem(data))?;
            let out = plan(format, &Mem(data), opts)?.to_vec(&Mem(data))?;
            // A damaged file can parse yet produce offsets that no longer resolve.
            parse_src(format, &Mem(&out))?;
            return Ok(out);
        }
    }
    let p = parse(format, data)?;
    let keep = exif::Keep {
        orientation: opts.keep_orientation,
        dates: opts.keep_dates,
        resolution: opts.keep_resolution,
    };
    let minimal_exif = if opts.exif {
        p.exif_tiff
            .as_deref()
            .and_then(|t| exif::preserved(t, keep))
    } else {
        None
    };
    match format {
        Format::Jpeg => jpeg::clean(data, opts, minimal_exif.as_deref()),
        Format::Png => png::clean(data, opts, minimal_exif.as_deref()),
        _ => webp::clean(data, opts, minimal_exif.as_deref()),
    }
}

/// Clean and verify the result by inspecting it again.
pub fn clean_and_verify(data: &[u8], opts: &CleanOptions) -> Result<(Vec<u8>, CleanReport)> {
    let before = inspect(data)?;
    let out = clean(data, opts)?;
    let after = inspect(&out)?;
    Ok((out, verify(before, after, opts)))
}

/// Inspects a file on disk. Audio and video are read in pieces, so files up
/// to [`MAX_STREAM_SIZE`] are accepted; other formats up to [`MAX_FILE_SIZE`].
pub fn inspect_path(path: &std::path::Path) -> Result<Inspection> {
    let (format, src) = open_path(path)?;
    match src {
        Some(src) => {
            let p = parse_src(format, &src)?;
            Ok(build(
                format,
                p,
                src.len() as usize,
                Input::Path(path),
                None,
            ))
        }
        None => inspect(&read_small(path)?),
    }
}

/// Cleans `input` into `output` and verifies the written file. The output is
/// written to a temporary file next to it and only moved into place when
/// verification passes; otherwise nothing is left behind. `output` may equal
/// `input` to clean in place.
pub fn clean_path(
    input: &std::path::Path,
    output: &std::path::Path,
    opts: &CleanOptions,
) -> Result<CleanReport> {
    let io = |e: std::io::Error| Error::Malformed(format!("{e}"));
    let (format, src) = open_path(input)?;
    let tmp = temp_name(output);
    let result = (|| -> Result<CleanReport> {
        let report = match src {
            Some(src) => {
                let before = build(
                    format,
                    parse_src(format, &src)?,
                    src.len() as usize,
                    Input::Path(input),
                    None,
                );
                let plan = plan(format, &src, opts)?;
                let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp).map_err(io)?);
                plan.write(&src, &mut w)?;
                use std::io::Write as _;
                w.flush().map_err(io)?;
                w.into_inner()
                    .map_err(|e| io(e.into_error()))?
                    .sync_all()
                    .map_err(io)?;
                drop(src);
                let after = inspect_path(&tmp)?;
                verify(before, after, opts)
            }
            None => {
                let data = read_small(input)?;
                let (out, report) = clean_and_verify(&data, opts)?;
                if report.verified {
                    std::fs::write(&tmp, &out).map_err(io)?;
                }
                report
            }
        };
        if report.verified {
            std::fs::rename(&tmp, output).map_err(io)?;
        }
        Ok(report)
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

fn temp_name(output: &std::path::Path) -> std::path::PathBuf {
    let name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    output.with_file_name(format!(".{name}.nexonic-tmp-{}", std::process::id()))
}

fn read_small(path: &std::path::Path) -> Result<Vec<u8>> {
    let len = std::fs::metadata(path)
        .map_err(|e| Error::Malformed(e.to_string()))?
        .len();
    if len > MAX_FILE_SIZE as u64 {
        return Err(Error::TooLarge);
    }
    std::fs::read(path).map_err(|e| Error::Malformed(e.to_string()))
}

/// Detects the format; returns a streaming source for audio and video.
fn open_path(path: &std::path::Path) -> Result<(Format, Option<FileSrc>)> {
    let src = FileSrc::open(path).map_err(|e| Error::Malformed(e.to_string()))?;
    let mut head = vec![0u8; src.len().min(4096) as usize];
    src.read_at(0, &mut head)?;
    let format = Format::detect(&head).ok_or(Error::Unsupported)?;
    if format.streams() {
        if src.len() > MAX_STREAM_SIZE {
            return Err(Error::TooLarge);
        }
        Ok((format, Some(src)))
    } else {
        Ok((format, None))
    }
}

fn verify(before: Inspection, after: Inspection, opts: &CleanOptions) -> CleanReport {
    // When EXIF is removed, only the tags the user chose to keep may remain.
    let allowed = exif::kept_tags(exif::Keep {
        orientation: opts.keep_orientation,
        dates: opts.keep_dates,
        resolution: opts.keep_resolution,
    });
    let mut still_present: Vec<Kind> = Vec::new();
    for b in &after.blocks {
        let preserved_exif = b.kind == Kind::Exif && !allowed.is_empty();
        if opts.removes(b.kind) && !preserved_exif && !still_present.contains(&b.kind) {
            still_present.push(b.kind);
        }
    }
    if opts.exif
        && after
            .details
            .iter()
            .any(|f| f.tag.is_some_and(|t| !allowed.contains(&t)))
        && !still_present.contains(&Kind::Exif)
    {
        still_present.push(Kind::Exif);
    }
    let mut notes = Vec::new();
    if before.credentials.present && !opts.c2pa {
        let other_removed = before
            .blocks
            .iter()
            .any(|b| b.kind != Kind::C2pa && opts.removes(b.kind));
        if other_removed && after.credentials.state == "invalid" {
            notes.push(msg("c2pa_broken", "Content Credentials were kept, but they no longer match the file because other metadata changed. Viewers will report them as invalid."));
        }
    }
    if before.credentials.present && opts.c2pa {
        notes.push(msg("c2pa_removed", "Embedded Content Credentials were removed from this file. Copies elsewhere, cloud records and platform labels are not affected."));
    }
    let image_unchanged = before.image_fingerprint == after.image_fingerprint;
    let mut problems = Vec::new();
    if !image_unchanged {
        problems.push(msg("image_changed", "The image data changed."));
    }
    if before.format != after.format
        || before.width != after.width
        || before.height != after.height
        || before.has_alpha != after.has_alpha
        || before.animated != after.animated
    {
        problems.push(msg(
            "structure_changed",
            "The format, dimensions, transparency or animation changed.",
        ));
    }
    if !still_present.is_empty() {
        problems.push(msg(
            "still_present",
            "Some selected metadata is still present.",
        ));
    }
    if !opts.embedded && before.embedded_fingerprint != after.embedded_fingerprint {
        problems.push(msg(
            "embedded_changed",
            "Embedded motion-photo or extra-image content changed.",
        ));
    }
    if before.has(Kind::Embedded) && !opts.embedded && opts.xmp && before.has(Kind::Xmp) {
        notes.push(msg("motion_xmp_removed", "The motion photo or extra images were kept, but XMP was removed. Some viewers use XMP to find them and may now show only the still image."));
    }
    if before.has(Kind::Embedded) && opts.embedded {
        notes.push(msg(
            "embedded_removed",
            "The embedded motion-photo video or extra images were removed.",
        ));
    }
    let verified = problems.is_empty();
    let bytes_removed = before.size as i64 - after.size as i64;
    CleanReport {
        before,
        after,
        image_unchanged,
        still_present,
        bytes_removed,
        verified,
        problems,
        notes,
    }
}

/// SHA-256 over image-data parts. Each part is length-prefixed so that
/// moving bytes between parts cannot produce the same digest.
pub(crate) struct Digest(sha2::Sha256);
impl Digest {
    pub fn new() -> Self {
        use sha2::Digest as _;
        Digest(sha2::Sha256::new())
    }
    pub fn add(&mut self, bytes: &[u8]) {
        use sha2::Digest as _;
        self.0.update((bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
    }
    pub fn finish(self) -> [u8; 32] {
        use sha2::Digest as _;
        self.0.finalize().into()
    }
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
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
