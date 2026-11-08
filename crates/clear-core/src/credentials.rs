//! Reads and validates embedded Content Credentials (C2PA) fully offline.
//! Results are reported as found; nothing is ever marked valid unless the
//! C2PA validator says so.

use crate::{Credentials, Format};
use c2pa::{Context, Reader, ValidationState};
use std::io::Cursor;

/// No network: no remote manifests, no OCSP. Trust lists are not bundled, so
/// a good signature from an unknown signer is reported as "valid", not "trusted".
const SETTINGS: &str = r#"{"verify":{"ocsp_fetch":false,"remote_manifest_fetch":false}}"#;

pub fn read(format: Format, data: &[u8]) -> Credentials {
    let mut c = Credentials {
        present: true,
        state: "unreadable".into(),
        ..Default::default()
    };
    let ctx = match Context::new().with_settings(SETTINGS) {
        Ok(ctx) => ctx,
        Err(e) => {
            c.problems.push(format!("Validator setup failed: {e}"));
            return c;
        }
    };
    let reader = match Reader::from_context(ctx).with_stream(format.mime(), Cursor::new(data)) {
        Ok(r) => r,
        Err(e) => {
            c.problems
                .push(format!("Could not read the credentials: {e}"));
            return c;
        }
    };
    c.state = match reader.validation_state() {
        ValidationState::Trusted => "trusted",
        ValidationState::Valid => "valid",
        _ => "invalid",
    }
    .into();
    c.manifests = reader.manifests().len();
    if let Some(list) = reader.validation_status() {
        for s in list.iter().take(20) {
            let code = s.code();
            // Untrusted-signer notices are expected without a bundled trust list.
            if code == "signingCredential.untrusted" {
                continue;
            }
            c.problems.push(match s.explanation() {
                Some(e) => format!("{code}: {e}"),
                None => code.to_string(),
            });
        }
    }
    if let Some(m) = reader.active_manifest() {
        c.title = m.title().map(str::to_owned);
        c.claim_generator = m
            .claim_generator_info
            .as_ref()
            .and_then(|v| v.first())
            .map(|g| match &g.version {
                Some(ver) => format!("{} {}", g.name, ver),
                None => g.name.clone(),
            })
            .or_else(|| m.claim_generator().map(str::to_owned));
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
