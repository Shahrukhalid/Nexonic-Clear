//! Native layer: image bytes arrive over raw IPC, are processed by clear-core,
//! and cleaned bytes are held until the user saves or discards them.
//! Nothing touches the network.

use clear_core::{CleanOptions, CleanReport, Inspection};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, Manager, State};

#[derive(Default)]
struct Settings(Mutex<Option<CleanOptions>>);

/// Options are sent by their own call: request headers are not delivered on every platform.
#[tauri::command]
fn set_options(options: CleanOptions, settings: State<'_, Settings>) {
    *settings.0.lock().unwrap() = Some(options);
}

#[derive(Default)]
struct Outputs {
    next: AtomicU64,
    files: Mutex<HashMap<u64, Vec<u8>>>,
}

#[derive(Serialize)]
struct Cleaned {
    id: u64,
    report: CleanReport,
}

/// Desktop webviews deliver a Uint8Array as a raw body. The Android webview
/// serialises it as JSON (an array, or an object keyed by index), so accept both.
fn body(request: &Request<'_>) -> Result<Vec<u8>, String> {
    let bad = || "Expected image bytes.".to_string();
    match request.body() {
        InvokeBody::Raw(data) => Ok(data.clone()),
        InvokeBody::Json(serde_json::Value::Array(items)) => items
            .iter()
            .map(|v| v.as_u64().filter(|b| *b <= 255).map(|b| b as u8))
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(bad),
        InvokeBody::Json(serde_json::Value::Object(map)) => {
            let mut out = vec![0u8; map.len()];
            for (k, v) in map {
                let i: usize = k.parse().map_err(|_| bad())?;
                let b = v.as_u64().filter(|b| *b <= 255).ok_or_else(bad)?;
                *out.get_mut(i).ok_or_else(bad)? = b as u8;
            }
            Ok(out)
        }
        _ => Err(bad()),
    }
    .and_then(|d| {
        if d.len() > clear_core::MAX_FILE_SIZE {
            Err("The file is larger than 256 MB.".into())
        } else {
            Ok(d)
        }
    })
}

#[tauri::command]
async fn inspect(request: Request<'_>) -> Result<Inspection, String> {
    let data = body(&request)?;
    clear_core::inspect(&data).map_err(|e| e.to_string())
}

#[tauri::command]
async fn clean(
    request: Request<'_>,
    outputs: State<'_, Outputs>,
    settings: State<'_, Settings>,
) -> Result<Cleaned, String> {
    let options = settings.0.lock().unwrap().clone().unwrap_or_default();
    let data = body(&request)?;
    let (out, report) = clear_core::clean_and_verify(&data, &options).map_err(|e| e.to_string())?;
    let id = outputs.next.fetch_add(1, Ordering::Relaxed);
    outputs.files.lock().unwrap().insert(id, out);
    Ok(Cleaned { id, report })
}

/// Cleaned bytes, kept until discarded so a file can be saved more than once.
#[tauri::command]
fn output(id: u64, outputs: State<'_, Outputs>) -> Result<Response, String> {
    let files = outputs.files.lock().unwrap();
    files
        .get(&id)
        .map(|d| Response::new(d.clone()))
        .ok_or_else(|| "This result is no longer available.".into())
}

#[tauri::command]
fn discard(ids: Vec<u64>, outputs: State<'_, Outputs>) {
    let mut files = outputs.files.lock().unwrap();
    for id in ids {
        files.remove(&id);
    }
}

// Native acceptance test: when a sample is placed in the self-test folder, the
// real interface runs the full inspect → clean → verify path over IPC and
// records the outcome. Desktop CI points NEXONIC_CLEAR_SELFTEST_DIR at a
// temporary folder; Android CI uses the app's private data folder.
fn selftest_dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = match std::env::var_os("NEXONIC_CLEAR_SELFTEST_DIR") {
        Some(d) => PathBuf::from(d),
        None => app.path().app_data_dir().ok()?.join("selftest"),
    };
    dir.join("sample.jpg").is_file().then_some(dir)
}

/// Empty when no self-test was requested.
#[tauri::command]
fn selftest_sample(app: AppHandle) -> Response {
    let data = selftest_dir(&app).and_then(|d| std::fs::read(d.join("sample.jpg")).ok());
    Response::new(data.unwrap_or_default())
}

#[tauri::command]
fn selftest_done(app: AppHandle, passed: bool, detail: String) {
    if let Some(dir) = selftest_dir(&app) {
        let result = serde_json::json!({ "passed": passed, "detail": detail });
        let _ = std::fs::write(dir.join("result.json"), result.to_string());
        let _ = std::fs::remove_file(dir.join("sample.jpg"));
    }
    if std::env::var_os("NEXONIC_CLEAR_SELFTEST_DIR").is_some() {
        app.exit(if passed { 0 } else { 1 });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(Outputs::default())
        .manage(Settings::default())
        .invoke_handler(tauri::generate_handler![
            inspect,
            set_options,
            clean,
            output,
            discard,
            selftest_sample,
            selftest_done
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Nexonic Clear");
}
