//! Phase 3: trust-list profiles, manifest inspection, `.c2pa` sidecars and
//! conformance results, on genuinely signed files.

use clear_core::credentials::{read_with_anchor, Source};
use clear_core::{inspect, inspect_with_sidecar, Format};
use std::io::Cursor;

fn fx(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn plain_jpeg() -> Vec<u8> {
    let img = image::RgbImage::from_fn(32, 24, |x, y| image::Rgb([x as u8 * 7, y as u8 * 9, 90]));
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .unwrap();
    out.into_inner()
}

/// Signs with the reference test certificate. With `sidecar`, returns the
/// untouched asset and the separate manifest bytes.
fn sign(data: &[u8], sidecar: bool, ingredient: Option<&[u8]>) -> (Vec<u8>, Vec<u8>) {
    let signer = c2pa::create_signer::from_keys(
        &fx("es256.pub"),
        &fx("es256.pem"),
        c2pa::SigningAlg::Es256,
        None,
    )
    .unwrap();
    let json = r#"{"claim_generator_info":[{"name":"phase3 tests","version":"1"}],"title":"edited","assertions":[{"label":"c2pa.actions","data":{"actions":[{"action":"c2pa.created","digitalSourceType":"http://cv.iptc.org/newscodes/digitalsourcetype/digitalCapture"}]}}]}"#;
    let mut b = c2pa::Builder::from_context(c2pa::Context::new())
        .with_definition(json)
        .unwrap();
    if let Some(parent) = ingredient {
        b.add_ingredient_from_stream(
            r#"{"title":"parent.jpg","relationship":"componentOf"}"#,
            "image/jpeg",
            &mut Cursor::new(parent.to_vec()),
        )
        .unwrap();
    }
    if sidecar {
        b.set_no_embed(true);
    }
    let mut out = Cursor::new(Vec::new());
    let manifest = b
        .sign(
            signer.as_ref(),
            "image/jpeg",
            &mut Cursor::new(data.to_vec()),
            &mut out,
        )
        .unwrap();
    let out = out.into_inner();
    (if out.is_empty() { data.to_vec() } else { out }, manifest)
}

fn test_root_pem() -> String {
    // Second certificate of the chain: the test intermediate CA.
    let chain = String::from_utf8(fx("es256.pub")).unwrap();
    let start = chain
        .match_indices("-----BEGIN CERTIFICATE-----")
        .nth(1)
        .unwrap()
        .0;
    chain[start..].to_string()
}

#[test]
fn bundled_trust_lists_do_not_trust_test_signer() {
    let (signed, _) = sign(&plain_jpeg(), false, None);
    let c = inspect(&signed).unwrap().credentials;
    assert_eq!(c.source, "embedded");
    assert_eq!(c.state, "valid", "{c:?}");
    let ids: Vec<_> = c.profiles.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["signature", "official", "interim"]);
    for p in &c.profiles {
        assert_eq!(p.state, "valid", "{p:?}");
    }
}

#[test]
fn trust_profile_reports_trusted_for_a_matching_anchor() {
    let (signed, _) = sign(&plain_jpeg(), false, None);
    let c = read_with_anchor(Format::Jpeg, &signed, Source::Embedded, &test_root_pem());
    let custom = c.profiles.iter().find(|p| p.id == "custom").unwrap();
    assert_eq!(custom.state, "trusted", "{c:?}");
    assert_eq!(c.state, "trusted");
    // An unrelated anchor must not grant trust.
    let other = std::fs::read_to_string(format!(
        "{}/trust/C2PA-TRUST-LIST.pem",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let c = read_with_anchor(Format::Jpeg, &signed, Source::Embedded, &other);
    assert_eq!(c.state, "valid", "{c:?}");
}

#[test]
fn manifest_store_lists_assertions_ingredients_and_codes() {
    let (parent, _) = sign(&plain_jpeg(), false, None);
    let (child, _) = sign(&parent, false, Some(&parent));
    let c = inspect(&child).unwrap().credentials;
    assert_eq!(c.manifests, 2, "{c:?}");
    assert_eq!(c.store.len(), 2);
    let active = &c.store[0];
    assert!(active.active);
    assert!(active.claim_version.is_some());
    assert!(active
        .assertions
        .iter()
        .any(|a| a.starts_with("c2pa.actions")));
    assert_eq!(active.ingredients.len(), 1);
    let ing = &active.ingredients[0];
    assert_eq!(ing.title.as_deref(), Some("parent.jpg"));
    assert_eq!(ing.relationship, "componentOf");
    assert_eq!(ing.manifest.as_deref(), Some(c.store[1].label.as_str()));
    assert!(
        active
            .success
            .iter()
            .any(|s| s == "claimSignature.validated"),
        "{active:?}"
    );
    assert!(active.failure.is_empty(), "{active:?}");
    assert!(!c.store[1].active);
}

#[test]
fn tampered_image_reports_failure_codes_in_every_profile() {
    let (mut signed, _) = sign(&plain_jpeg(), false, None);
    let n = signed.len();
    signed[n - 40] ^= 0x55;
    let c = inspect(&signed).unwrap().credentials;
    assert_eq!(c.state, "invalid");
    for p in &c.profiles {
        assert_eq!(p.state, "invalid");
        assert!(p.failures.iter().any(|f| f.contains("mismatch")), "{p:?}");
    }
    assert!(c.store[0]
        .failure
        .iter()
        .any(|f| f.starts_with("assertion.dataHash.mismatch")));
}

#[test]
fn sidecar_manifest_validates_against_its_image() {
    let image = plain_jpeg();
    let (asset, manifest) = sign(&image, true, None);
    assert_eq!(asset, image, "sidecar signing must not change the image");
    assert!(!inspect(&asset).unwrap().credentials.present);
    let c = inspect_with_sidecar(&asset, &manifest).unwrap().credentials;
    assert!(c.present);
    assert_eq!(c.source, "sidecar");
    assert_eq!(c.state, "valid", "{c:?}");
    // The same sidecar against a different image must fail.
    let mut other = image.clone();
    let k = other.len() - 30;
    other[k] ^= 0x0f;
    let c = inspect_with_sidecar(&other, &manifest).unwrap().credentials;
    assert_eq!(c.state, "invalid", "{c:?}");
}

#[test]
fn malformed_sidecars_never_panic_or_validate() {
    let image = plain_jpeg();
    let (_, manifest) = sign(&image, true, None);
    let mut cases: Vec<Vec<u8>> = vec![vec![], b"not a manifest".to_vec(), vec![0; 4096]];
    for cut in [8, 64, manifest.len() / 2, manifest.len() - 1] {
        cases.push(manifest[..cut].to_vec());
    }
    let mut seed = 0x1234_5678u32;
    for _ in 0..40 {
        let mut m = manifest.clone();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let i = seed as usize % m.len();
            m[i] ^= (seed >> 8) as u8 | 1;
        }
        cases.push(m);
    }
    for m in cases {
        let c = inspect_with_sidecar(&image, &m).unwrap().credentials;
        assert_ne!(c.state, "trusted");
        if c.state == "valid" {
            // A flipped byte may land in an unhashed area; signature must still verify.
            assert!(c.problems.is_empty(), "{c:?}");
        }
    }
    assert!(inspect_with_sidecar(&image, &vec![0; clear_core::MAX_SIDECAR + 1]).is_err());
}

#[test]
fn real_fixtures_report_profiles_and_store() {
    for name in ["CA.jpg", "E-sig-CA.jpg"] {
        let c = inspect(&fx(name)).unwrap().credentials;
        assert!(c.present, "{name}");
        assert_eq!(c.profiles.len(), 3, "{name}");
        assert_eq!(c.store.len(), c.manifests, "{name}");
        assert!(c.store.first().map(|m| m.active).unwrap_or(false), "{name}");
    }
}
