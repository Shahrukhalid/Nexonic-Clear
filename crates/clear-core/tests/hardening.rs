//! Regression tests for the hardening pass: malformed input, embedded content
//! preservation, multi-picture offsets, strict PNG/WebP validation.

use clear_core::{clean, clean_and_verify, inspect, CleanOptions, Error, Kind};

fn fx(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn strip_all() -> CleanOptions {
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
    }
}

/// A plain JPEG with no metadata, derived from a real file.
fn bare_jpeg() -> Vec<u8> {
    clean(&fx("no_manifest.jpg"), &strip_all()).unwrap()
}

fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0xFF, marker];
    v.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    v.extend_from_slice(payload);
    v
}

fn insert_after_soi(jpeg: &[u8], segs: &[Vec<u8>]) -> Vec<u8> {
    let mut v = jpeg[..2].to_vec();
    for s in segs {
        v.extend_from_slice(s);
    }
    v.extend_from_slice(&jpeg[2..]);
    v
}

fn xmp(text: &str) -> Vec<u8> {
    let mut p = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    p.extend_from_slice(text.as_bytes());
    segment(0xE1, &p)
}

#[test]
fn short_exif_payload_does_not_panic() {
    let d = insert_after_soi(&bare_jpeg(), &[segment(0xE1, b"Exif\0")]);
    let i = inspect(&d).expect("a short EXIF block is reported, not a crash");
    assert!(i.has(Kind::Exif));
    assert!(i.exif.is_none());
    let (_, r) = clean_and_verify(&d, &CleanOptions::default()).unwrap();
    assert!(r.verified && !r.after.has(Kind::Exif));
}

#[test]
fn image_fingerprint_is_sha256_and_distinguishes_images() {
    let a = inspect(&fx("IMG_0003.jpg")).unwrap();
    let b = inspect(&fx("CA.jpg")).unwrap();
    assert_eq!(a.image_fingerprint.len(), 64);
    assert_ne!(a.image_fingerprint, b.image_fingerprint);
}

#[test]
fn motion_photo_video_is_kept_by_default_and_removable() {
    // A real-shaped MP4 start: box size, "ftyp", brand.
    let mut video = vec![0, 0, 0, 0x18];
    video.extend_from_slice(b"ftypmp42\0\0\0\0mp42isom");
    video.extend(std::iter::repeat_n(0xAB, 4096));
    let mut d = insert_after_soi(
        &bare_jpeg(),
        &[xmp("<x:xmpmeta GCamera:MotionPhoto=\"1\"/>")],
    );
    d.extend_from_slice(&video);
    let before = inspect(&d).unwrap();
    assert!(before.has(Kind::Embedded));
    assert!(before.embedded_fingerprint.is_some());

    let (out, r) = clean_and_verify(&d, &CleanOptions::default()).unwrap();
    assert!(r.verified, "{:?}", r.problems);
    assert!(out.ends_with(&video), "video must be byte-identical");
    assert_eq!(r.after.embedded_fingerprint, before.embedded_fingerprint);
    assert!(r.notes.iter().any(|n| n.contains("XMP was removed")));

    let opts = CleanOptions {
        embedded: true,
        ..CleanOptions::default()
    };
    let (out, r) = clean_and_verify(&d, &opts).unwrap();
    assert!(r.verified && !r.after.has(Kind::Embedded));
    assert!(!out.ends_with(&video));
    // "Other" removal alone must never drop the video.
    let opts = CleanOptions {
        other: true,
        embedded: false,
        ..CleanOptions::default()
    };
    assert!(clean(&d, &opts).unwrap().ends_with(&video));
}

/// Builds a JPEG with an MPF index pointing at a second image after EOI.
fn mpf_jpeg(extra_after_index: &[Vec<u8>]) -> (Vec<u8>, Vec<u8>) {
    let second = bare_jpeg();
    let build = |offset: u32| {
        let mut t = b"MPF\0MM\0*\0\0\0\x08".to_vec();
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0xB002u16.to_be_bytes());
        t.extend_from_slice(&7u16.to_be_bytes());
        t.extend_from_slice(&32u32.to_be_bytes());
        t.extend_from_slice(&26u32.to_be_bytes());
        t.extend_from_slice(&0u32.to_be_bytes());
        // Entry 1: primary image (offset 0). Entry 2: the appended image.
        t.extend_from_slice(&[0x20, 0x03, 0, 0]);
        t.extend_from_slice(&0u32.to_be_bytes());
        t.extend_from_slice(&0u32.to_be_bytes());
        t.extend_from_slice(&[0; 4]);
        t.extend_from_slice(&[0, 0x02, 0, 0]);
        t.extend_from_slice(&(second.len() as u32).to_be_bytes());
        t.extend_from_slice(&offset.to_be_bytes());
        t.extend_from_slice(&[0; 4]);
        let mut segs = vec![segment(0xE2, &t)];
        segs.extend_from_slice(extra_after_index);
        let mut d = insert_after_soi(&bare_jpeg(), &segs);
        let tiff_at = 2 + 8;
        let off = (d.len() - tiff_at) as u32;
        d.extend_from_slice(&second);
        (d, off)
    };
    let (_, off) = build(0);
    let (d, _) = build(off);
    (d, second)
}

fn mpf_second_offset(d: &[u8]) -> usize {
    let tiff = 2 + 8;
    let entry2 = tiff + 26 + 16;
    tiff + u32::from_be_bytes(d[entry2 + 8..entry2 + 12].try_into().unwrap()) as usize
}

#[test]
fn multi_picture_offsets_follow_removed_segments() {
    let (d, second) = mpf_jpeg(&[xmp("<x:xmpmeta/>"), segment(0xFE, b"comment")]);
    assert_eq!(&d[mpf_second_offset(&d)..], &second[..]);
    let (out, r) = clean_and_verify(&d, &CleanOptions::default()).unwrap();
    assert!(r.verified, "{:?}", r.problems);
    assert!(out.len() < d.len());
    assert_eq!(
        &out[mpf_second_offset(&out)..],
        &second[..],
        "offset must point at the second image"
    );
    assert!(r.after.has(Kind::Embedded));
}

#[test]
fn png_checksum_and_structure_are_enforced() {
    let png = fx("libpng-test.png");
    let mut bad = png.clone();
    let n = bad.len();
    bad[n - 20] ^= 0x01; // inside the last IDAT, so its CRC no longer matches
    assert!(matches!(inspect(&bad), Err(Error::Malformed(_))));
    // IHDR must come first.
    let mut swapped = png[..8].to_vec();
    swapped.extend_from_slice(&png[n - 12..]);
    assert!(inspect(&swapped).is_err());
}

#[test]
fn trailing_data_is_reported_and_kept_unless_removed() {
    for name in ["libpng-test.png", "test.webp"] {
        let mut d = fx(name);
        d.extend_from_slice(b"HIDDEN-PAYLOAD");
        let i = inspect(&d).unwrap();
        assert!(
            i.blocks
                .iter()
                .any(|b| b.label == "Data after end of image"),
            "{name}"
        );
        let keep = CleanOptions {
            other: false,
            ..CleanOptions::default()
        };
        assert!(
            clean(&d, &keep).unwrap().ends_with(b"HIDDEN-PAYLOAD"),
            "{name}"
        );
        let (out, r) = clean_and_verify(&d, &CleanOptions::default()).unwrap();
        assert!(r.verified && !out.ends_with(b"HIDDEN-PAYLOAD"), "{name}");
    }
}

#[test]
fn webp_structure_is_enforced() {
    let d = fx("test_xmp.webp");
    // RIFF size larger than the file.
    let mut big = d.clone();
    big[4..8].copy_from_slice(&((d.len() as u32) + 100).to_le_bytes());
    assert!(inspect(&big).is_err());
    // A file with metadata chunks but no image bitstream.
    let mut empty = b"RIFF\0\0\0\0WEBP".to_vec();
    empty.extend_from_slice(b"EXIF\x04\0\0\0abcd");
    let size = (empty.len() - 8) as u32;
    empty[4..8].copy_from_slice(&size.to_le_bytes());
    assert!(inspect(&empty).is_err());
}

#[test]
fn every_real_fixture_verifies_after_default_cleaning() {
    for name in [
        "IMG_0003.jpg",
        "no_manifest.jpg",
        "CA.jpg",
        "libpng-test.png",
        "test_xmp.webp",
        "mars.webp",
        "test.webp",
    ] {
        let (_, r) = clean_and_verify(&fx(name), &CleanOptions::default()).unwrap();
        assert!(r.verified, "{name}: {:?}", r.problems);
    }
}
