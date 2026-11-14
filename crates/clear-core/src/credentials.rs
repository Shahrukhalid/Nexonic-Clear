//! Reads and validates Content Credentials (C2PA) fully offline.
//!
//! Credentials are validated once per profile:
//! - `signature`: cryptographic signature, hashes and structure, with no trust list;
//! - `official`: the bundled C2PA trust list (signers and time-stamping authorities);
//! - `interim`: the bundled, frozen interim trust list.
//!
//! Nothing is reported as trusted unless the validator says so for a bundled list.

use crate::{Credentials, Format, IngredientInfo, Input, ManifestInfo, ProfileResult};
use c2pa::{Context, Reader, ValidationState};
use std::io::Cursor;

trait ReadSeek: std::io::Read + std::io::Seek + Send {}
impl<T: std::io::Read + std::io::Seek + Send> ReadSeek for T {}

const OFFICIAL_SIGNERS: &str = include_str!("../trust/C2PA-TRUST-LIST.pem");
const OFFICIAL_TSA: &str = include_str!("../trust/C2PA-TSA-TRUST-LIST.pem");
const INTERIM_ANCHORS: &str = include_str!("../trust/interim-anchors.pem");
const INTERIM_ALLOWED: &str = include_str!("../trust/interim-allowed.pem");
const INTERIM_CONFIG: &str = include_str!("../trust/store.cfg");

/// Where the manifest came from.
#[derive(Clone, Copy)]
pub enum Source<'a> {
    Embedded,
    Sidecar(&'a [u8]),
}

fn anchor(pem: &str, uri: &str, kind: &str, config: Option<&str>, allowed: Option<&str>) -> String {
    let mut a = format!(
        r#"{{"trust_anchors":{},"trust_uri":{},"trust_kind":{}"#,
        json_str(pem),
        json_str(uri),
        json_str(kind)
    );
    if let Some(c) = config {
        a.push_str(&format!(r#","trust_config":{}"#, json_str(c)));
    }
    if let Some(l) = allowed {
        a.push_str(&format!(r#","allowed_list":{}"#, json_str(l)));
    }
    a.push('}');
    a
}

fn settings(anchors: &[String]) -> String {
    let trust = if anchors.is_empty() {
        String::new()
    } else {
        format!(r#","trust":{{"anchors":[{}]}}"#, anchors.join(","))
    };
    format!(
        r#"{{"verify":{{"ocsp_fetch":false,"remote_manifest_fetch":false,"verify_trust":{}}}{trust}}}"#,
        !anchors.is_empty()
    )
}

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn profiles() -> Vec<(&'static str, String)> {
    vec![
        ("signature", settings(&[])),
        (
            "official",
            settings(&[
                anchor(OFFICIAL_SIGNERS, "c2pa-trust-list", "manifest", None, None),
                anchor(OFFICIAL_TSA, "c2pa-tsa-trust-list", "tsa", None, None),
            ]),
        ),
        (
            "interim",
            settings(&[anchor(
                INTERIM_ANCHORS,
                "interim-trust-list",
                "manifest",
                Some(INTERIM_CONFIG),
                Some(INTERIM_ALLOWED),
            )]),
        ),
    ]
}

fn open(settings: &str, format: Format, data: Input, source: Source) -> Result<Reader, String> {
    let ctx = Context::new()
        .with_settings(settings)
        .map_err(|e| format!("Validator setup failed: {e}"))?;
    let reader = Reader::from_context(ctx);
    let mut file;
    let mut cursor;
    let stream: &mut dyn ReadSeek = match data {
        Input::Bytes(b) => {
            cursor = Cursor::new(b);
            &mut cursor
        }
        Input::Path(p) => {
            file = std::fs::File::open(p).map_err(|e| format!("Could not open the file: {e}"))?;
            &mut file
        }
    };
    match source {
        Source::Embedded => reader.with_stream(format.mime(), stream),
        Source::Sidecar(m) => reader.with_manifest_data_and_stream(m, format.mime(), stream),
    }
    .map_err(|e| format!("Could not read the credentials: {e}"))
}

fn state_name(s: ValidationState) -> &'static str {
    match s {
        ValidationState::Trusted => "trusted",
        ValidationState::Valid => "valid",
        _ => "invalid",
    }
}

fn status_text(s: &c2pa::validation_status::ValidationStatus) -> String {
    match s.explanation() {
        Some(e) => format!("{}: {e}", s.code()),
        None => s.code().to_string(),
    }
}

fn is_trust_notice(code: &str) -> bool {
    code == "signingCredential.untrusted" || code == "timeStamp.untrusted"
}

/// Validates with each bundled profile. `extra` adds a caller-supplied trust
/// anchor profile (used by tests with the test signing root).
pub(crate) fn read(format: Format, data: Input, source: Source) -> Credentials {
    read_profiles(format, data, source, profiles())
}

#[doc(hidden)]
pub fn read_with_anchor(format: Format, data: &[u8], source: Source, pem: &str) -> Credentials {
    let data = Input::Bytes(data);
    let mut p = profiles();
    p.truncate(1);
    p.push((
        "custom",
        settings(&[anchor(
            pem,
            "custom",
            "manifest",
            Some(INTERIM_CONFIG),
            None,
        )]),
    ));
    read_profiles(format, data, source, p)
}

fn read_profiles(
    format: Format,
    data: Input,
    source: Source,
    list: Vec<(&'static str, String)>,
) -> Credentials {
    let mut c = Credentials {
        present: true,
        state: "unreadable".into(),
        source: match source {
            Source::Embedded => "embedded",
            Source::Sidecar(_) => "sidecar",
        }
        .into(),
        ..Default::default()
    };
    let mut base: Option<Reader> = None;
    for (id, s) in list {
        match open(&s, format, data, source) {
            Ok(r) => {
                let state = state_name(r.validation_state());
                let failures = r
                    .validation_results()
                    .and_then(|v| v.active_manifest())
                    .map(|a| a.failure().iter().map(status_text).collect())
                    .unwrap_or_default();
                c.profiles.push(ProfileResult {
                    id: id.into(),
                    state: state.into(),
                    failures,
                });
                if base.is_none() {
                    base = Some(r);
                }
            }
            Err(e) => {
                c.profiles.push(ProfileResult {
                    id: id.into(),
                    state: "unreadable".into(),
                    failures: vec![e.clone()],
                });
                if c.problems.is_empty() {
                    c.problems.push(e);
                }
            }
        }
    }
    let Some(reader) = base else { return c };

    // Overall state: trusted only when a trust-list profile says trusted.
    c.state = if c
        .profiles
        .iter()
        .any(|p| p.id != "signature" && p.state == "trusted")
    {
        "trusted".into()
    } else {
        state_name(reader.validation_state()).into()
    };
    c.manifests = reader.manifests().len();
    if let Some(list) = reader.validation_status() {
        for s in list.iter().take(20) {
            if !is_trust_notice(s.code()) {
                c.problems.push(status_text(s));
            }
        }
    }

    let active_label = reader.active_label().map(str::to_owned);
    let active_codes = reader
        .validation_results()
        .and_then(|v| v.active_manifest());
    let mut labels: Vec<&String> = reader.manifests().keys().collect();
    labels.sort();
    // Active manifest first, then the rest in a stable order.
    labels.sort_by_key(|l| Some(l.as_str()) != active_label.as_deref());
    for label in labels.into_iter().take(50) {
        let Some(m) = reader.get_manifest(label) else {
            continue;
        };
        let active = Some(label.as_str()) == active_label.as_deref();
        let sig = m.signature_info();
        let mut info = ManifestInfo {
            label: label.clone(),
            active,
            claim_version: m.claim_version(),
            title: m.title().map(str::to_owned),
            format: m.format().map(str::to_owned),
            generator: generator(m),
            issuer: sig.and_then(|s| s.issuer.clone().or_else(|| s.common_name.clone())),
            signed_at: sig.and_then(|s| s.time.clone()),
            algorithm: sig.and_then(|s| s.alg.map(|a| a.to_string())),
            assertions: m
                .assertions()
                .iter()
                .take(200)
                .map(|a| a.label_with_instance())
                .collect(),
            ingredients: m
                .ingredients()
                .iter()
                .take(100)
                .map(|i| IngredientInfo {
                    title: i.title().map(str::to_owned),
                    format: i.format().map(str::to_owned),
                    relationship: i.relationship().as_str().to_string(),
                    manifest: i.active_manifest().map(str::to_owned),
                    failure: i
                        .validation_status()
                        .unwrap_or_default()
                        .iter()
                        .filter(|s| {
                            !c2pa::validation_results::validation_codes::is_success(s.code())
                                && !is_trust_notice(s.code())
                        })
                        .map(status_text)
                        .collect(),
                })
                .collect(),
            ..Default::default()
        };
        if active {
            if let Some(a) = active_codes {
                info.success = a.success().iter().map(|s| s.code().to_string()).collect();
                info.informational = a.informational().iter().map(status_text).collect();
                info.failure = a.failure().iter().map(status_text).collect();
            }
        }
        c.store.push(info);
    }

    if let Some(m) = reader.active_manifest() {
        c.title = m.title().map(str::to_owned);
        c.claim_generator = generator(m);
        if let Some(sig) = m.signature_info() {
            c.issuer = sig.issuer.clone().or_else(|| sig.common_name.clone());
            c.signed_at = sig.time.clone();
            c.algorithm = sig.alg.map(|a| a.to_string());
        }
        for a in m.assertions() {
            if !a.label().starts_with("c2pa.actions") {
                continue;
            }
            let Ok(v) = a.value() else { continue };
            for act in v
                .get("actions")
                .and_then(|x| x.as_array())
                .into_iter()
                .flatten()
            {
                if let Some(name) = act.get("action").and_then(|x| x.as_str()) {
                    let name = name.trim_start_matches("c2pa.").to_string();
                    if !c.actions.contains(&name) {
                        c.actions.push(name);
                    }
                }
                let dst = act
                    .get("digitalSourceType")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                if dst.ends_with("trainedAlgorithmicMedia")
                    || dst.ends_with("compositeWithTrainedAlgorithmicMedia")
                    || dst.ends_with("algorithmicMedia")
                {
                    c.ai_generated = true;
                }
            }
        }
    }
    c
}

fn generator(m: &c2pa::Manifest) -> Option<String> {
    m.claim_generator_info
        .as_ref()
        .and_then(|v| v.first())
        .map(|g| match &g.version {
            Some(ver) => format!("{} {}", g.name, ver),
            None => g.name.clone(),
        })
        .or_else(|| m.claim_generator().map(str::to_owned))
}
