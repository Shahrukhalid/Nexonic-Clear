//! Audio, video, HEIC/AVIF and PDF on real files made with ffmpeg, libheif,
//! libavif and qpdf (see THIRD_PARTY_NOTICES). Decoded media is compared with
//! ffmpeg before and after cleaning; CI sets NEXONIC_REQUIRE_FFMPEG=1 so the
//! comparison can't be skipped there.

use clear_core::{
    clean, clean_and_verify, clean_path, inspect, inspect_path, CleanOptions, Format, Kind,
};
use std::io::Cursor;
use std::process::Command;

fn fx(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

const MEDIA: &[(&str, Format)] = &[
    ("media.wav", Format::Wav),
    ("media.mp3", Format::Mp3),
    ("media.mp4", Format::Mp4),
    ("media.mov", Format::Mov),
    ("media.m4a", Format::M4a),
    ("media.heic", Format::Heif),
    ("media.avif", Format::Avif),
    ("media.pdf", Format::Pdf),
];

fn all() -> CleanOptions {
    CleanOptions {
        exif: true,
        xmp: true,
        iptc: true,
        icc: true,
        c2pa: true,
        comments: true,
        other: true,
        embedded: true,
        keep_orientation: false,
        keep_dates: false,
        keep_resolution: false,
    }
}

fn none() -> CleanOptions {
    CleanOptions {
        exif: false,
        xmp: false,
        iptc: false,
        icc: false,
        c2pa: false,
        comments: false,
        other: false,
        embedded: false,
        keep_orientation: false,
        keep_dates: false,
        keep_resolution: false,
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("nexonic-p4-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

/// MD5 of every decoded stream, or None when ffmpeg is missing.
fn decoded(data: &[u8], ext: &str) -> Option<String> {
    let path = tmp(&format!("decode-{}.{ext}", data.len()));
    std::fs::write(&path, data).unwrap();
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        // MP3 cover art is removable content; compare the audio only.
        .args([
            "-map",
            if ext == "mp3" { "0:a" } else { "0" },
            "-f",
            "streamhash",
            "-hash",
            "md5",
            "-",
        ])
        .output();
    let _ = std::fs::remove_file(&path);
    match out {
        Ok(o) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).into()),
        // Many ffmpeg builds can't read HEIF still images; item data is
        // covered by the fingerprint instead.
        Ok(_) if ext == "heic" || ext == "avif" => None,
        Ok(o) => panic!(
            "ffmpeg could not decode {ext}: {}",
            String::from_utf8_lossy(&o.stderr)
        ),
        Err(_) => {
            assert!(
                std::env::var("NEXONIC_REQUIRE_FFMPEG").is_err(),
                "ffmpeg is required"
            );
            None
        }
    }
}

#[test]
fn metadata_is_found() {
    for (name, format) in MEDIA {
        let i = inspect(&fx(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(i.format, *format, "{name}");
        assert!(!i.blocks.is_empty(), "{name}: no metadata found");
        let text: String = i
            .details
            .iter()
            .map(|f| format!("{}={};", f.name, f.value))
            .collect();
        assert!(text.contains("Secret"), "{name}: details missing: {text}");
    }
    let mov = inspect(&fx("media.mov")).unwrap();
    assert!(mov.has(Kind::Exif), "QuickTime location/make/model");
    assert!(mov.details.iter().any(|f| f.name == "Location"));
    let heic = inspect(&fx("media.heic")).unwrap();
    assert!(heic.has(Kind::Exif) && heic.has(Kind::Xmp));
    // libheif stores the coded size (64x64) with a crop to 64x48.
    assert_eq!(heic.width, 64);
    assert!(heic.exif.as_ref().is_some_and(|e| e.has_gps));
    assert!(heic.details.iter().any(|f| f.value.contains("SecretMake")));
    let avif = inspect(&fx("media.avif")).unwrap();
    assert!(avif.has(Kind::Exif) && avif.has(Kind::Xmp));
    assert_eq!((avif.width, avif.height), (64, 48));
    let mp4 = inspect(&fx("media.mp4")).unwrap();
    assert_eq!((mp4.width, mp4.height), (64, 48));
    let mp3 = inspect(&fx("media.mp3")).unwrap();
    assert!(mp3.has(Kind::Embedded), "cover art");
    let pdf = inspect(&fx("media.pdf")).unwrap();
    for k in [Kind::Comment, Kind::Xmp, Kind::Other, Kind::Embedded] {
        assert!(pdf.has(k), "pdf {k:?}");
    }
    assert_eq!((pdf.width, pdf.height), (200, 100));
}

#[test]
fn removing_everything_leaves_no_secrets_and_media_is_identical() {
    for (name, format) in MEDIA {
        let data = fx(name);
        let (out, r) = clean_and_verify(&data, &all()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            r.verified,
            "{name}: {:?} still {:?}",
            r.problems, r.still_present
        );
        assert!(
            r.after.blocks.is_empty(),
            "{name}: left {:?}",
            r.after.blocks
        );
        // MP3 keeps the encoder name inside its first audio frame (LAME header).
        let lavf: &[u8] = if *format == Format::Mp3 {
            b"Secret"
        } else {
            b"Lavf"
        };
        for s in [&b"Secret"[..], lavf, b"+12.3456"] {
            assert!(
                !contains(&out, s),
                "{name}: {} still in output",
                String::from_utf8_lossy(s)
            );
        }
        assert!(out.len() < data.len(), "{name}");
        if *format == Format::Pdf {
            assert!(contains(&out, b"Visible text"));
            continue;
        }
        let ext = format.extension();
        if let Some(before) = decoded(&data, ext) {
            assert_eq!(
                Some(before),
                decoded(&out, ext),
                "{name}: decoded media changed"
            );
        }
    }
}

#[test]
fn faststart_offsets_are_moved() {
    // moov is before mdat, so removing metadata from moov shifts every chunk.
    let data = fx("media.mp4");
    let moov = data.windows(4).position(|w| w == b"moov").unwrap();
    let mdat = data.windows(4).position(|w| w == b"mdat").unwrap();
    assert!(moov < mdat);
    let (out, r) = clean_and_verify(&data, &all()).unwrap();
    assert!(r.verified && r.bytes_removed > 0);
    assert_eq!(r.before.image_fingerprint, r.after.image_fingerprint);
    // A wrong offset would hash different bytes: corrupt one stco entry and check.
    let mut bad = out.clone();
    let p = bad.windows(4).position(|w| w == b"stco").unwrap();
    bad[p + 15] ^= 1;
    assert_ne!(
        inspect(&bad)
            .map(|i| i.image_fingerprint)
            .unwrap_or_default(),
        r.after.image_fingerprint
    );
}

#[test]
fn keeping_a_kind_keeps_it() {
    for (name, _) in MEDIA {
        let data = fx(name);
        let before = inspect(&data).unwrap();
        let (out, r) = clean_and_verify(&data, &none()).unwrap();
        assert!(r.verified, "{name}");
        assert_eq!(before.blocks.len(), r.after.blocks.len(), "{name}");
        if !matches!(*name, "media.pdf") {
            assert_eq!(
                out, data,
                "{name}: nothing selected must not change the file"
            );
        }
        for kind in [
            Kind::Exif,
            Kind::Xmp,
            Kind::Comment,
            Kind::Other,
            Kind::Embedded,
        ] {
            if !before.has(kind) {
                continue;
            }
            let mut o = all();
            match kind {
                Kind::Exif => o.exif = false,
                Kind::Xmp => o.xmp = false,
                Kind::Comment => o.comments = false,
                Kind::Other => o.other = false,
                _ => o.embedded = false,
            }
            let (_, r) = clean_and_verify(&data, &o).unwrap();
            assert!(r.verified, "{name} keep {kind:?}: {:?}", r.problems);
            assert!(r.after.has(kind), "{name}: {kind:?} was not kept");
            assert!(
                r.after.blocks.iter().all(|b| b.kind == kind),
                "{name}: {:?}",
                r.after.blocks
            );
        }
    }
}

#[test]
fn pdf_rewrite_drops_old_revisions() {
    let data = fx("media.pdf");
    assert!(contains(&data, b"OldSecretAuthor"));
    let mut o = none();
    o.xmp = true;
    let (out, r) = clean_and_verify(&data, &o).unwrap();
    assert!(r.verified);
    assert!(
        !contains(&out, b"OldSecretAuthor"),
        "an older revision survived"
    );
    assert!(contains(&out, b"SecretAuthor"), "kept document information");
    assert!(!contains(&out, b"SecretXmpTool"));
}

fn sign(data: &[u8], mime: &str) -> Vec<u8> {
    let signer = c2pa::create_signer::from_keys(
        &fx("es256.pub"),
        &fx("es256.pem"),
        c2pa::SigningAlg::Es256,
        None,
    )
    .unwrap();
    let json = r#"{"claim_generator_info":[{"name":"Nexonic Clear tests","version":"1"}],"title":"fixture","assertions":[{"label":"c2pa.actions","data":{"actions":[{"action":"c2pa.created","digitalSourceType":"http://cv.iptc.org/newscodes/digitalsourcetype/digitalCapture"}]}}]}"#;
    let mut b = c2pa::Builder::from_context(c2pa::Context::new())
        .with_definition(json)
        .unwrap();
    let mut out = Cursor::new(Vec::new());
    b.sign(
        signer.as_ref(),
        mime,
        &mut Cursor::new(data.to_vec()),
        &mut out,
    )
    .unwrap();
    out.into_inner()
}

#[test]
fn signed_media_is_read_removed_and_kept() {
    for (name, format) in MEDIA {
        let signed = sign(&fx(name), format.mime());
        let i = inspect(&signed).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            i.has(Kind::C2pa),
            "{name}: manifest not found in {:?}",
            i.blocks
        );
        assert_eq!(
            i.credentials.state, "valid",
            "{name}: {:?}",
            i.credentials.problems
        );
        // Remove only the credentials.
        let mut o = none();
        o.c2pa = true;
        let (out, r) = clean_and_verify(&signed, &o).unwrap();
        assert!(
            r.verified && !r.after.has(Kind::C2pa),
            "{name}: {:?}",
            r.problems
        );
        assert!(!r.after.credentials.present, "{name}");
        assert!(out.len() < signed.len());
        // Keeping everything leaves them valid.
        let (_, r) = clean_and_verify(&signed, &none()).unwrap();
        assert_eq!(r.after.credentials.state, "valid", "{name}");
    }
}

#[test]
fn path_api_matches_memory_and_never_writes_unverified() {
    for (name, _) in MEDIA {
        let data = fx(name);
        let src = tmp(&format!("in-{name}"));
        let dst = tmp(&format!("out-{name}"));
        std::fs::write(&src, &data).unwrap();
        let a = inspect_path(&src).unwrap();
        assert_eq!(
            a.image_fingerprint,
            inspect(&data).unwrap().image_fingerprint,
            "{name}"
        );
        let r = clean_path(&src, &dst, &all()).unwrap();
        assert!(r.verified, "{name}");
        assert_eq!(
            std::fs::read(&dst).unwrap(),
            clean(&data, &all()).unwrap(),
            "{name}"
        );
        let leftovers = std::fs::read_dir(dst.parent().unwrap())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("nexonic-tmp")
            })
            .count();
        assert_eq!(leftovers, 0);
    }
}

#[test]
fn large_wav_is_cleaned_by_streaming() {
    // 300 MB: above the in-memory limit, handled from disk.
    let src = tmp("big.wav");
    let dst = tmp("big-clean.wav");
    {
        use std::io::Write;
        let data: u64 = 300 * 1024 * 1024;
        let info = b"LIST\x1a\x00\x00\x00INFOINAM\x0d\x00\x00\x00SecretTitle!\x00\x00";
        let mut f = std::io::BufWriter::new(std::fs::File::create(&src).unwrap());
        let total = 4 + 24 + info.len() as u64 + 8 + data;
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(total as u32).to_le_bytes()).unwrap();
        f.write_all(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x02\x00\x44\xac\x00\x00\x10\xb1\x02\x00\x04\x00\x10\x00").unwrap();
        f.write_all(info).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&(data as u32).to_le_bytes()).unwrap();
        let block = vec![7u8; 1 << 20];
        for _ in 0..300 {
            f.write_all(&block).unwrap();
        }
    }
    assert!(
        inspect(&std::fs::read(&src).unwrap()).is_err(),
        "too large for memory"
    );
    let i = inspect_path(&src).unwrap();
    assert!(i.has(Kind::Comment));
    let r = clean_path(&src, &dst, &all()).unwrap();
    assert!(r.verified && r.after.blocks.is_empty());
    assert_eq!(
        std::fs::metadata(&dst).unwrap().len(),
        std::fs::metadata(&src).unwrap().len() - 34
    );
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&dst);
}

#[test]
fn damaged_media_never_panics() {
    for (name, _) in MEDIA {
        let data = fx(name);
        for cut in (0..data.len()).step_by((data.len() / 200).max(1)) {
            let d = &data[..cut];
            if inspect(d).is_ok() {
                let _ = clean_and_verify(d, &all());
            }
        }
        let mut seed = 0x2545F4914F6CDD1Du64;
        for _ in 0..300 {
            let mut d = data.clone();
            for _ in 0..4 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let i = (seed as usize) % d.len();
                d[i] = (seed >> 32) as u8;
            }
            if inspect(&d).is_ok() {
                let _ = clean_and_verify(&d, &all());
            }
        }
    }
}
