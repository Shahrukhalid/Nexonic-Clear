//! Detailed inspection and selective EXIF preservation on real files.

use clear_core::{clean, clean_and_verify, inspect, CleanOptions, Kind};

fn fx(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn has(details: &[clear_core::Field], group: &str, name: &str, value: &str) -> bool {
    details
        .iter()
        .any(|f| f.group == group && f.name == name && f.value.contains(value))
}

#[test]
fn details_cover_exif_xmp_iptc_and_icc() {
    let i = inspect(&fx("IMG_0003.jpg")).unwrap();
    assert!(has(
        &i.details,
        "EXIF",
        "Camera model",
        "Canon EOS REBEL T3"
    ));
    assert!(has(
        &i.details,
        "EXIF",
        "Camera serial number",
        "062062059949"
    ));
    assert!(has(&i.details, "EXIF", "Date taken", "2011:07:30"));
    assert!(has(&i.details, "XMP", "xmp:CreatorTool", "Lightroom"));
    assert!(has(&i.details, "IPTC", "Date created", "20110730"));
    assert!(has(&i.details, "ICC", "Colour space", "RGB"));
    let w = inspect(&fx("test_xmp.webp")).unwrap();
    assert!(has(&w.details, "XMP", "dc:title", "This is a test"));
    assert!(has(&w.details, "XMP", "dc:subject", "C2PA; Authenticity"));
}

#[test]
fn removing_everything_leaves_no_details() {
    let opts = CleanOptions {
        icc: true,
        c2pa: true,
        embedded: true,
        keep_orientation: false,
        keep_dates: false,
        keep_resolution: false,
        ..CleanOptions::default()
    };
    let (_, r) = clean_and_verify(&fx("IMG_0003.jpg"), &opts).unwrap();
    assert!(r.verified, "{:?}", r.problems);
    assert!(r.after.details.is_empty(), "{:?}", r.after.details);
}

#[test]
fn dates_and_resolution_are_kept_only_when_chosen() {
    let src = fx("IMG_0003.jpg");
    // Default: resolution kept, dates and everything else gone.
    let (_, r) = clean_and_verify(&src, &CleanOptions::default()).unwrap();
    assert!(r.verified, "{:?}", r.problems);
    let exif: Vec<_> = r.after.details.iter().filter(|f| f.tag.is_some()).collect();
    assert!(exif
        .iter()
        .any(|f| f.name == "X resolution" && f.value == "240"));
    assert!(!exif.iter().any(|f| f.name.starts_with("Date")), "{exif:?}");
    assert!(
        !exif.iter().any(|f| f.name.starts_with("Camera")),
        "{exif:?}"
    );
    assert!(!r.after.details.iter().any(|f| f.group == "GPS"));

    let keep = CleanOptions {
        keep_dates: true,
        ..CleanOptions::default()
    };
    let (_, r) = clean_and_verify(&src, &keep).unwrap();
    assert!(r.verified, "{:?}", r.problems);
    assert!(has(
        &r.after.details,
        "EXIF",
        "Date taken",
        "2011:07:30 15:41:25"
    ));
    assert!(has(&r.after.details, "EXIF", "Date modified", "2022:05:24"));
    assert!(!r
        .after
        .details
        .iter()
        .any(|f| f.name == "Camera serial number"));

    let none = CleanOptions {
        keep_resolution: false,
        keep_orientation: false,
        ..CleanOptions::default()
    };
    let (_, r) = clean_and_verify(&src, &none).unwrap();
    assert!(r.verified && !r.after.has(Kind::Exif));
}

#[test]
fn preserved_exif_is_stable_and_parses_in_every_format() {
    for name in ["IMG_0003.jpg", "test_xmp.webp", "no_manifest.jpg"] {
        let keep = CleanOptions {
            keep_dates: true,
            ..CleanOptions::default()
        };
        let once = clean(&fx(name), &keep).unwrap();
        assert_eq!(clean(&once, &keep).unwrap(), once, "{name}");
        let (_, r) = clean_and_verify(&fx(name), &keep).unwrap();
        assert!(r.verified, "{name}: {:?}", r.problems);
    }
}
