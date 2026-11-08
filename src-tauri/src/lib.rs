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
struct Outputs {
    next: AtomicU64,
    files: Mutex<HashMap<u64, Vec<u8>>>,
}

#[derive(Serialize)]
struct Cleaned {
    id: u64,
    report: CleanReport,
}

fn body(request: &Request<'_>) -> Result<Vec<u8>, String> {
    match request.body() {
        InvokeBody::Raw(data) => Ok(data.clone()),
        _ => Err("Expected image bytes.".into()),
    }
}

#[tauri::command]
async fn inspect(request: Request<'_>) -> Result<Inspection, String> {
    let data = body(&request)?;
    clear_core::inspect(&data).map_err(|e| e.to_string())
}

#[tauri::command]
async fn clean(request: Request<'_>, outputs: State<'_, Outputs>) -> Result<Cleaned, String> {
    let options: CleanOptions = match request.headers().get("x-options") {
        Some(v) => {
            serde_json::from_slice(v.as_bytes()).map_err(|_| "Invalid options.".to_string())?
        }
        None => CleanOptions::default(),
    };
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
        .invoke_handler(tauri::generate_handler![
            inspect,
            clean,
            output,
            discard,
            selftest_sample,
            selftest_done
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Nexonic Clear");
}
