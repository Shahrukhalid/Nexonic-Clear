//! Functional tests on real image files, including genuinely signed C2PA
//! content. Fixtures from the C2PA reference SDK (MIT) are in tests/fixtures.

use clear_core::{clean, clean_and_verify, inspect, CleanOptions, Error, Format, Kind};
use std::io::Cursor;

fn fx(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Decoded pixels, to prove cleaning never changes the picture.
fn pixels(data: &[u8]) -> (u32, u32, Vec<u8>) {
    let img = image::load_from_memory(data).expect("output must decode");
    (img.width(), img.height(), img.to_rgba8().into_raw())
}

/// Sign an image with the C2PA reference test certificate (genuine signature,
/// untrusted signer), returning the signed bytes.
fn sign(data: &[u8], mime: &str, ai: bool) -> Vec<u8> {
    let cert = fx("es256.pub");
    let key = fx("es256.pem");
    let signer =
        c2pa::create_signer::from_keys(&cert, &key, c2pa::SigningAlg::Es256, None).unwrap();
    let dst = if ai {
        r#","digitalSourceType":"http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia""#
    } else {
        r#","digitalSourceType":"http://cv.iptc.org/newscodes/digitalsourcetype/digitalCapture""#
    };
    let json = format!(
        r#"{{"claim_generator_info":[{{"name":"Nexonic Clear tests","version":"1"}}],"title":"fixture","assertions":[{{"label":"c2pa.actions","data":{{"actions":[{{"action":"c2pa.created"{dst}}}]}}}}]}}"#
    );
    let mut b = c2pa::Builder::from_context(c2pa::Context::new())
        .with_definition(json.as_str())
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
    }
}

#[test]
fn jpeg_detects_exif_xmp_iptc_icc() {
    let d = fx("IMG_0003.jpg");
    let i = inspect(&d).unwrap();
    assert_eq!(i.format, Format::Jpeg);
    for k in [Kind::Exif, Kind::Xmp, Kind::Iptc, Kind::Icc] {
        assert!(i.has(k), "missing {k:?}");
    }
    let e = i.exif.expect("exif summary");
    assert!(e.tag_count > 0);
    assert!(
        e.fields.iter().any(|(k, _)| k == "Camera"),
        "{:?}",
        e.fields
    );
    assert!(!i.credentials.present);
}

#[test]
fn jpeg_default_clean_keeps_icc_and_pixels() {
    let d = fx("IMG_0003.jpg");
    let (out, r) = clean_and_verify(&d, &CleanOptions::default()).unwrap();
    assert!(r.still_present.is_empty(), "{:?}", r.still_present);
    assert!(r.image_unchanged);
    assert!(!r.after.has(Kind::Xmp) && !r.after.has(Kind::Iptc));
    assert!(
        r.after.has(Kind::Icc),
        "colour profile must be kept by default"
    );
    assert!(r.bytes_removed > 0);
    assert_eq!(pixels(&d), pixels(&out));
}

#[test]
fn jpeg_remove_everything() {
    let d = fx("no_manifest.jpg");
    let (out, r) = clean_and_verify(&d, &all()).unwrap();
    assert!(r.after.blocks.is_empty(), "{:?}", r.after.blocks);
    assert!(r.image_unchanged);
    assert_eq!(pixels(&d), pixels(&out));
}

#[test]
fn signed_jpeg_is_valid_and_tampered_is_invalid() {
    let ok = inspect(&fx("CA.jpg")).unwrap();
    assert!(ok.credentials.present);
    assert!(
        ok.credentials.state == "valid" || ok.credentials.state == "trusted",
        "{:?}",
        ok.credentials
    );
    assert!(ok.credentials.issuer.is_some() || ok.credentials.claim_generator.is_some());
    let bad = inspect(&fx("E-sig-CA.jpg")).unwrap();
    assert!(bad.credentials.present);
    assert_eq!(bad.credentials.state, "invalid");
    assert!(!bad.credentials.problems.is_empty());
}

#[test]
fn remove_c2pa_from_jpeg() {
    let d = fx("CA.jpg");
    let o = CleanOptions {
        c2pa: true,
        ..CleanOptions::default()
    };
    let (out, r) = clean_and_verify(&d, &o).unwrap();
    assert!(!r.after.has(Kind::C2pa));
    assert!(!r.after.credentials.present);
    assert!(r.image_unchanged);
    assert_eq!(pixels(&d), pixels(&out));
}

#[test]
fn keeping_c2pa_while_editing_other_metadata_is_reported_honestly() {
    let signed = sign(&fx("IMG_0003.jpg"), "image/jpeg", false);
    let before = inspect(&signed).unwrap();
    assert_eq!(
        before.credentials.state, "valid",
        "{:?}",
        before.credentials
    );
    // Keep only the credentials untouched: nothing else removed -> still valid.
    let none = CleanOptions {
        exif: false,
        xmp: false,
        iptc: false,
        icc: false,
        c2pa: false,
        comments: false,
        other: false,
        embedded: false,
        keep_orientation: false,
    };
    let (_, r) = clean_and_verify(&signed, &none).unwrap();
    assert_eq!(r.after.credentials.state, "valid");
    // Removing other metadata breaks the hash binding; the report must say so.
    let (_, r) = clean_and_verify(&signed, &CleanOptions::default()).unwrap();
    assert!(r.after.credentials.present);
    assert_eq!(r.after.credentials.state, "invalid");
    assert!(!r.notes.is_empty());
}

#[test]
fn ai_generation_flag_is_read_from_credentials() {
    let signed = sign(&fx("no_manifest.jpg"), "image/jpeg", true);
    let i = inspect(&signed).unwrap();
    assert!(i.credentials.ai_generated);
    assert!(i.credentials.actions.contains(&"created".to_string()));
    let plain = sign(&fx("no_manifest.jpg"), "image/jpeg", false);
    assert!(!inspect(&plain).unwrap().credentials.ai_generated);
}

#[test]
fn png_signed_detect_and_remove() {
    let d = fx("libpng-test.png");
    let signed = sign(&d, "image/png", false);
    let i = inspect(&signed).unwrap();
    assert_eq!(i.format, Format::Png);
    assert!(i.has(Kind::C2pa));
    assert_eq!(i.credentials.state, "valid", "{:?}", i.credentials);
    let (out, r) = clean_and_verify(
        &signed,
        &CleanOptions {
            c2pa: true,
            ..CleanOptions::default()
        },
    )
    .unwrap();
    assert!(!r.after.has(Kind::C2pa));
    assert!(r.image_unchanged);
    assert_eq!(pixels(&signed), pixels(&out));
    assert_eq!(pixels(&d), pixels(&out));
}

#[test]
fn webp_xmp_removed_flags_fixed_icc_kept() {
    let d = fx("test_xmp.webp");
    let i = inspect(&d).unwrap();
    assert!(i.has(Kind::Xmp) && i.has(Kind::Icc));
    let (out, r) = clean_and_verify(&d, &CleanOptions::default()).unwrap();
    assert!(!r.after.has(Kind::Xmp));
    assert!(r.after.has(Kind::Icc));
    assert!(r.image_unchanged);
    // VP8X XMP flag must be cleared, ICC flag kept.
    assert_eq!(&out[12..16], b"VP8X");
    assert_eq!(out[20] & 0x04, 0);
    assert_ne!(out[20] & 0x20, 0);
    assert_eq!(pixels(&d), pixels(&out));
    let (out2, _) = clean_and_verify(&d, &all()).unwrap();
    assert_eq!(out2[20] & 0x24, 0);
    assert_eq!(pixels(&d), pixels(&out2));
}

#[test]
fn webp_signed_detect_and_remove() {
    let d = fx("test.webp");
    let signed = sign(&d, "image/webp", false);
    let i = inspect(&signed).unwrap();
    assert!(i.has(Kind::C2pa), "{:?}", i.blocks);
    assert_eq!(i.credentials.state, "valid", "{:?}", i.credentials);
    let (out, r) = clean_and_verify(&signed, &all()).unwrap();
    assert!(!r.after.has(Kind::C2pa) && !r.after.credentials.present);
    assert!(r.image_unchanged);
    assert_eq!(pixels(&d), pixels(&out));
}

/// Build a JPEG with orientation 6 and GPS from a clean file.
fn with_orientation_and_gps(base: &[u8]) -> Vec<u8> {
    // Big-endian TIFF: IFD0 {Make, Orientation=6, GPS pointer}, GPS IFD {LatRef N, Lat 10/1 30/1 0/1}.
    let mut t = Vec::new();
    t.extend_from_slice(b"MM\0*\0\0\0\x08");
    t.extend_from_slice(&3u16.to_be_bytes());
    let entry = |t: &mut Vec<u8>, tag: u16, ty: u16, count: u32, val: [u8; 4]| {
        t.extend_from_slice(&tag.to_be_bytes());
        t.extend_from_slice(&ty.to_be_bytes());
        t.extend_from_slice(&count.to_be_bytes());
        t.extend_from_slice(&val);
    };
    entry(&mut t, 0x010F, 2, 4, *b"Acm\0");
    entry(&mut t, 0x0112, 3, 1, [0, 6, 0, 0]);
    entry(&mut t, 0x8825, 4, 1, 50u32.to_be_bytes());
    t.extend_from_slice(&0u32.to_be_bytes()); // next IFD; now at offset 50
    assert_eq!(t.len(), 50);
    t.extend_from_slice(&4u16.to_be_bytes());
    entry(&mut t, 1, 2, 2, *b"N\0\0\0");
    entry(&mut t, 2, 5, 3, 104u32.to_be_bytes());
    entry(&mut t, 3, 2, 2, *b"W\0\0\0");
    entry(&mut t, 4, 5, 3, 128u32.to_be_bytes());
    t.extend_from_slice(&0u32.to_be_bytes());
    assert_eq!(t.len(), 104);
    for (n, d) in [(10u32, 1u32), (30, 1), (0, 1), (20, 1), (15, 1), (0, 1)] {
        t.extend_from_slice(&n.to_be_bytes());
        t.extend_from_slice(&d.to_be_bytes());
    }
    let mut out = base[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((t.len() + 8) as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&t);
    out.extend_from_slice(&base[2..]);
    out
}

#[test]
fn gps_shown_and_orientation_kept() {
    let base = clean(&fx("no_manifest.jpg"), &all()).unwrap();
    let d = with_orientation_and_gps(&base);
    let i = inspect(&d).unwrap();
    let e = i.exif.unwrap();
    assert!(e.has_gps);
    assert!(
        e.fields
            .iter()
            .any(|(k, v)| k == "Location" && v == "10.50000, -20.25000"),
        "{:?}",
        e.fields
    );
    let (_, r) = clean_and_verify(&d, &CleanOptions::default()).unwrap();
    assert!(r.still_present.is_empty(), "{:?}", r.still_present);
    let e = r.after.exif.expect("orientation-only exif");
    assert_eq!(e.fields, vec![("Orientation".to_string(), "6".to_string())]);
    assert!(!e.has_gps);
    assert!(r.image_unchanged);
    let (_, r) = clean_and_verify(
        &d,
        &CleanOptions {
            keep_orientation: false,
            ..CleanOptions::default()
        },
    )
    .unwrap();
    assert!(r.after.exif.is_none());
}

#[test]
fn unsupported_and_malformed_inputs_never_panic() {
    assert_eq!(inspect(b"GIF89a....").unwrap_err(), Error::Unsupported);
    assert_eq!(inspect(&[]).unwrap_err(), Error::Unsupported);
    for name in [
        "IMG_0003.jpg",
        "CA.jpg",
        "libpng-test.png",
        "test_xmp.webp",
        "mars.webp",
        "test.webp",
    ] {
        let d = fx(name);
        let step = (d.len() / 300).max(1);
        let mut n = 0;
        while n < d.len() {
            let cut = &d[..n];
            let _ = inspect(cut);
            let _ = clean(cut, &all());
            n += step;
        }
        // Corrupt a length field region.
        let mut bad = d.clone();
        for b in bad.iter_mut().skip(20).take(16) {
            *b = 0xFF;
        }
        let _ = inspect(&bad);
        let _ = clean(&bad, &all());
    }
}

#[test]
fn cleaning_is_idempotent() {
    for name in ["IMG_0003.jpg", "test_xmp.webp", "libpng-test.png"] {
        let once = clean(&fx(name), &CleanOptions::default()).unwrap();
        let twice = clean(&once, &CleanOptions::default()).unwrap();
        assert_eq!(once, twice, "{name}");
    }
}
