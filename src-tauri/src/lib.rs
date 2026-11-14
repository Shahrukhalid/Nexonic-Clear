//! Native layer. Desktop reads files by path; Android sends bytes over IPC.
//! Cleaned results are written to private temporary files (not held in
//! memory) and can only be exported when verification passed. Desktop saves
//! are atomic: data goes to a temporary file in the destination folder, is
//! flushed, and is then renamed into place. Nothing touches the network.

use clear_core::{CleanOptions, CleanReport, Format, Inspection};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
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

struct Output {
    file: PathBuf,
    /// The original file, so its modification time can be carried over.
    source: Option<PathBuf>,
    verified: bool,
    format: Format,
}

#[derive(Default)]
struct Outputs {
    next: AtomicU64,
    files: Mutex<HashMap<u64, Output>>,
}

#[derive(Serialize)]
struct Cleaned {
    id: u64,
    report: CleanReport,
}

// Errors cross IPC as "code|message" so the interface can translate them.
const TOO_LARGE: &str = "too_large|The file is too large.";
const UNVERIFIED: &str = "unverified|This result did not pass verification and cannot be saved.";

fn core_err(e: clear_core::Error) -> String {
    format!("{}|{e}", e.code())
}

/// Desktop webviews deliver a Uint8Array as a raw body. The Android webview
/// serialises it as JSON (an array, or an object keyed by index), so accept both.
fn body(request: &Request<'_>) -> Result<Vec<u8>, String> {
    let bad = || "malformed|Expected image bytes.".to_string();
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
            Err(TOO_LARGE.into())
        } else {
            Ok(d)
        }
    })
}

/// Reads a file chosen by the user (dialog or drag and drop), refusing
/// anything that is not a supported image.
fn read_image(path: &str) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("io|Could not open the file: {e}"))?;
    if !meta.is_file() {
        return Err("io|Not a file.".into());
    }
    if meta.len() > clear_core::MAX_FILE_SIZE as u64 {
        return Err(TOO_LARGE.into());
    }
    let data = std::fs::read(path).map_err(|e| format!("io|Could not read the file: {e}"))?;
    if Format::detect(&data).is_none() {
        return Err(core_err(clear_core::Error::Unsupported));
    }
    Ok(data)
}

fn output_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|_| "io|No private storage available.".to_string())?
        .join("outputs");
    std::fs::create_dir_all(&dir).map_err(|e| format!("io|Could not prepare storage: {e}"))?;
    Ok(dir)
}

fn ext(format: Format) -> &'static str {
    format.extension()
}

/// Audio and video above the in-memory limit are processed from disk.
fn is_large(path: &str) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > clear_core::MAX_FILE_SIZE as u64)
}

/// Cleans a large file straight from disk into private storage. The result
/// is written only when verification passes.
fn run_clean_large(
    app: &AppHandle,
    outputs: &Outputs,
    settings: &Settings,
    path: &str,
) -> Result<Cleaned, String> {
    let options = settings.0.lock().unwrap().clone().unwrap_or_default();
    let id = outputs.next.fetch_add(1, Ordering::Relaxed);
    let probe = clear_core::inspect_path(Path::new(path)).map_err(core_err)?;
    let file = output_dir(app)?.join(format!("{id}.{}", ext(probe.format)));
    let report = clear_core::clean_path(Path::new(path), &file, &options).map_err(core_err)?;
    outputs.files.lock().unwrap().insert(
        id,
        Output {
            file,
            source: Some(PathBuf::from(path)),
            verified: report.verified,
            format: report.after.format,
        },
    );
    Ok(Cleaned { id, report })
}

fn store(
    app: &AppHandle,
    outputs: &Outputs,
    data: &[u8],
    report: &CleanReport,
    source: Option<PathBuf>,
) -> Result<u64, String> {
    let id = outputs.next.fetch_add(1, Ordering::Relaxed);
    let format = report.after.format;
    let file = output_dir(app)?.join(format!("{id}.{}", ext(format)));
    std::fs::write(&file, data).map_err(|e| format!("io|Could not store the result: {e}"))?;
    outputs.files.lock().unwrap().insert(
        id,
        Output {
            file,
            source,
            verified: report.verified,
            format,
        },
    );
    Ok(id)
}

fn run_clean(
    app: &AppHandle,
    outputs: &Outputs,
    settings: &Settings,
    data: &[u8],
    source: Option<PathBuf>,
) -> Result<Cleaned, String> {
    let options = settings.0.lock().unwrap().clone().unwrap_or_default();
    let (out, report) = clear_core::clean_and_verify(data, &options).map_err(core_err)?;
    let id = store(app, outputs, &out, &report, source)?;
    Ok(Cleaned { id, report })
}

#[tauri::command]
async fn inspect(request: Request<'_>) -> Result<Inspection, String> {
    clear_core::inspect(&body(&request)?).map_err(core_err)
}

#[tauri::command]
async fn inspect_path(path: String) -> Result<Inspection, String> {
    if is_large(&path) {
        return clear_core::inspect_path(Path::new(&path)).map_err(core_err);
    }
    let data = read_image(&path)?;
    match sidecar_for(Path::new(&path)) {
        Some(m) => clear_core::inspect_with_sidecar(&data, &m),
        None => clear_core::inspect(&data),
    }
    .map_err(core_err)
}

/// A separate Content Credentials file next to the image: `photo.c2pa` or
/// `photo.jpg.c2pa`. Symbolic links and oversized files are ignored.
fn sidecar_for(image: &Path) -> Option<Vec<u8>> {
    let mut candidates = vec![image.with_extension("c2pa")];
    if let Some(name) = image.file_name() {
        let mut n = name.to_os_string();
        n.push(".c2pa");
        candidates.push(image.with_file_name(n));
    }
    for c in candidates {
        let Ok(meta) = std::fs::symlink_metadata(&c) else {
            continue;
        };
        if !meta.is_file() || meta.len() as usize > clear_core::MAX_SIDECAR {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&c) {
            return Some(bytes);
        }
    }
    None
}

#[tauri::command]
async fn clean(
    app: AppHandle,
    request: Request<'_>,
    outputs: State<'_, Outputs>,
    settings: State<'_, Settings>,
) -> Result<Cleaned, String> {
    run_clean(&app, &outputs, &settings, &body(&request)?, None)
}

#[tauri::command]
async fn clean_path(
    app: AppHandle,
    path: String,
    outputs: State<'_, Outputs>,
    settings: State<'_, Settings>,
) -> Result<Cleaned, String> {
    if is_large(&path) {
        return run_clean_large(&app, &outputs, &settings, &path);
    }
    run_clean(
        &app,
        &outputs,
        &settings,
        &read_image(&path)?,
        Some(PathBuf::from(&path)),
    )
}

/// Original image bytes for the preview thumbnail (supported images only).
#[tauri::command]
async fn read_path(path: String) -> Result<Response, String> {
    read_image(&path).map(Response::new)
}

/// Files shared into the app from another app (Android share sheet). The
/// Android activity copies them into `<cache>/shared`; each call moves them to
/// a new folder and returns their paths, so every file is returned once.
#[tauri::command]
async fn take_shared(app: AppHandle) -> Vec<String> {
    let Ok(cache) = app.path().app_cache_dir() else {
        return Vec::new();
    };
    take_shared_from(&cache)
}

fn take_shared_from(cache: &Path) -> Vec<String> {
    let inbox = cache.join("shared");
    let Ok(rd) = std::fs::read_dir(&inbox) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    if files.is_empty() {
        return Vec::new();
    }
    files.sort();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let taken = cache.join("shared-taken").join(stamp.to_string());
    if std::fs::create_dir_all(&taken).is_err() {
        return Vec::new();
    }
    files
        .into_iter()
        .filter_map(|f| {
            let to = taken.join(f.file_name()?);
            std::fs::rename(&f, &to).ok()?;
            Some(to.to_string_lossy().into_owned())
        })
        .collect()
}

/// Expands dropped or chosen folders into supported image files, recursively.
/// Symbolic links are not followed; at most 20,000 files are returned.
fn expand(paths: Vec<String>) -> Vec<String> {
    const LIMIT: usize = 20_000;
    let supported = |p: &Path| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| Format::EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
    };
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    stack.reverse();
    while let Some(p) = stack.pop() {
        if out.len() >= LIMIT {
            break;
        }
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_file() {
            if supported(&p) {
                out.push(p.to_string_lossy().into_owned());
            }
        } else if meta.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&p) {
                let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
                children.sort();
                children.reverse();
                stack.extend(children);
            }
        }
    }
    out
}

#[tauri::command]
async fn expand_paths(paths: Vec<String>) -> Vec<String> {
    expand(paths)
}

struct Ready {
    file: PathBuf,
    source: Option<PathBuf>,
    format: Format,
}

fn verified_file(outputs: &Outputs, id: u64) -> Result<Ready, String> {
    let files = outputs.files.lock().unwrap();
    let o = files
        .get(&id)
        .ok_or("gone|This result is no longer available.")?;
    if !o.verified {
        return Err(UNVERIFIED.into());
    }
    Ok(Ready {
        file: o.file.clone(),
        source: o.source.clone(),
        format: o.format,
    })
}

/// Cleaned bytes for platforms that save through the webview (Android).
#[tauri::command]
async fn output(id: u64, outputs: State<'_, Outputs>) -> Result<Response, String> {
    let ready = verified_file(&outputs, id)?;
    std::fs::read(ready.file)
        .map(Response::new)
        .map_err(|e| format!("io|{e}"))
}

/// Writes `data` to `dest` atomically: temporary file in the same folder,
/// flushed to disk, then renamed. The destination never holds partial data.
/// When `time_from` is given, the file's modification time is copied from it.
fn atomic_write(data: &[u8], dest: &Path, time_from: Option<&Path>) -> Result<(), String> {
    let dir = dest.parent().ok_or("io|Invalid destination.")?;
    let name = dest
        .file_name()
        .ok_or("io|Invalid destination.")?
        .to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.part", std::process::id()));
    let modified = time_from
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok());
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        if let Some(t) = modified {
            f.set_modified(t)?;
        }
        drop(f);
        std::fs::rename(&tmp, dest)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("io|Could not save: {e}"))
}

fn atomic_copy(src: &Path, dest: &Path, time_from: Option<&Path>) -> Result<(), String> {
    let data = std::fs::read(src).map_err(|e| format!("io|Could not save: {e}"))?;
    atomic_write(&data, dest, time_from)
}

#[derive(Serialize)]
struct Saved {
    id: u64,
    name: String,
}

/// Desktop: asks where to save one result, then writes it atomically.
#[tauri::command]
async fn save_one(
    app: AppHandle,
    id: u64,
    name: String,
    keep_time: bool,
    outputs: State<'_, Outputs>,
) -> Result<Option<String>, String> {
    #[cfg(mobile)]
    {
        let _ = (app, id, name, keep_time, outputs);
        return Err("io|Saving this way is available on desktop only.".into());
    }
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt;
        let ready = verified_file(&outputs, id)?;
        let Some(dest) = app
            .dialog()
            .file()
            .set_file_name(&name)
            .add_filter("Image", &[ext(ready.format)])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let dest = dest
            .into_path()
            .map_err(|_| "io|Unsupported destination.".to_string())?;
        let time_from = ready.source.as_deref().filter(|_| keep_time);
        atomic_copy(&ready.file, &dest, time_from)?;
        Ok(dest.file_name().map(|n| n.to_string_lossy().into_owned()))
    }
}

/// Desktop: asks for a folder and saves every verified result there,
/// choosing new names so nothing is overwritten. Unverified results are skipped.
#[tauri::command]
async fn save_all(
    app: AppHandle,
    items: Vec<(u64, String)>,
    keep_time: bool,
    outputs: State<'_, Outputs>,
) -> Result<Option<(String, Vec<Saved>)>, String> {
    #[cfg(mobile)]
    {
        let _ = (app, items, keep_time, outputs);
        return Err("io|Saving this way is available on desktop only.".into());
    }
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt;
        let Some(dir) = app.dialog().file().blocking_pick_folder() else {
            return Ok(None);
        };
        let dir = dir
            .into_path()
            .map_err(|_| "io|Unsupported folder.".to_string())?;
        let mut saved = Vec::new();
        for (id, name) in items {
            let Ok(ready) = verified_file(&outputs, id) else {
                continue;
            };
            let safe: String = name
                .chars()
                .map(|c| {
                    if c.is_control() || "/\\:*?\"<>|".contains(c) {
                        '_'
                    } else {
                        c
                    }
                })
                .collect();
            let (stem, extn) = match safe.rsplit_once('.') {
                Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
                _ => (safe.clone(), String::new()),
            };
            let mut dest = dir.join(&safe);
            let mut n = 2;
            while dest.exists() {
                dest = dir.join(format!("{stem}-{n}{extn}"));
                n += 1;
            }
            let time_from = ready.source.as_deref().filter(|_| keep_time);
            atomic_copy(&ready.file, &dest, time_from)?;
            saved.push(Saved {
                id,
                name: dest.file_name().unwrap().to_string_lossy().into_owned(),
            });
        }
        Ok(Some((dir.to_string_lossy().into_owned(), saved)))
    }
}

/// Desktop: saves a JSON report atomically to a location the user chooses.
#[tauri::command]
async fn save_report(app: AppHandle, json: String, name: String) -> Result<Option<String>, String> {
    #[cfg(mobile)]
    {
        let _ = (app, json, name);
        return Err("io|Saving this way is available on desktop only.".into());
    }
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt;
        serde_json::from_str::<serde_json::Value>(&json)
            .map_err(|_| "malformed|Invalid report.".to_string())?;
        let Some(dest) = app
            .dialog()
            .file()
            .set_file_name(&name)
            .add_filter("JSON report", &["json"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let dest = dest
            .into_path()
            .map_err(|_| "io|Unsupported destination.".to_string())?;
        atomic_write(json.as_bytes(), &dest, None)?;
        Ok(dest.file_name().map(|n| n.to_string_lossy().into_owned()))
    }
}

#[tauri::command]
fn discard(ids: Vec<u64>, outputs: State<'_, Outputs>) {
    let mut files = outputs.files.lock().unwrap();
    for id in ids {
        if let Some(o) = files.remove(&id) {
            let _ = std::fs::remove_file(o.file);
        }
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

/// Path of the self-test sample, so the desktop path-based flow is exercised too.
#[tauri::command]
fn selftest_path(app: AppHandle) -> Option<String> {
    selftest_dir(&app).map(|d| d.join("sample.jpg").to_string_lossy().into_owned())
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
        .setup(|app| {
            // Results from a previous session are never reused.
            if let Ok(dir) = output_dir(app.handle()) {
                let _ = std::fs::remove_dir_all(&dir);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            take_shared,
            inspect,
            inspect_path,
            set_options,
            clean,
            clean_path,
            read_path,
            expand_paths,
            output,
            save_one,
            save_all,
            save_report,
            discard,
            selftest_sample,
            selftest_path,
            selftest_done
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Nexonic Clear");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_sidecar_next_to_image() {
        let dir = std::env::temp_dir().join(format!("clear-sidecar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.jpg");
        let b = dir.join("b.png");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        assert!(sidecar_for(&a).is_none());
        std::fs::write(dir.join("a.c2pa"), b"one").unwrap();
        std::fs::write(dir.join("b.png.c2pa"), b"two").unwrap();
        assert_eq!(sidecar_for(&a).as_deref(), Some(&b"one"[..]));
        assert_eq!(sidecar_for(&b).as_deref(), Some(&b"two"[..]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("clear-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn expand_recurses_filters_and_skips_symlinks() {
        let d = tmp("expand");
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        for f in [
            "x.JPG",
            "a/y.png",
            "a/b/z.webp",
            "a/notes.txt",
            "a/b/w.jpeg",
        ] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&d, d.join("a/loop")).unwrap();
        let got = expand(vec![d.to_string_lossy().into_owned()]);
        let names: Vec<_> = got
            .iter()
            .map(|p| {
                Path::new(p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names.len(), 4, "{names:?}");
        for n in ["x.JPG", "y.png", "z.webp", "w.jpeg"] {
            assert!(names.contains(&n.to_string()), "{n} missing from {names:?}");
        }
    }

    #[test]
    fn atomic_copy_replaces_whole_file_and_leaves_no_temp_files() {
        let d = tmp("atomic");
        let src = d.join("src.bin");
        let dest = d.join("out.jpg");
        std::fs::write(&src, vec![7u8; 100_000]).unwrap();
        std::fs::write(&dest, b"old contents").unwrap();
        atomic_copy(&src, &dest, Some(&src)).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), vec![7u8; 100_000]);
        let t = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(t(&dest), t(&src), "modification time carried over");
        let leftovers: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(leftovers.is_empty());
        // A failing write leaves the destination untouched and cleans up.
        assert!(atomic_copy(&d.join("missing.bin"), &dest, None).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), vec![7u8; 100_000]);
    }
}

#[cfg(test)]
mod shared_tests {
    #[test]
    fn shared_files_are_returned_once() {
        let cache = std::env::temp_dir().join(format!("nexonic-share-{}", std::process::id()));
        std::fs::create_dir_all(cache.join("shared")).unwrap();
        std::fs::write(cache.join("shared/a.jpg"), b"x").unwrap();
        std::fs::write(cache.join("shared/b.mp4"), b"y").unwrap();
        let got = super::take_shared_from(&cache);
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|p| std::path::Path::new(p).is_file()));
        assert!(super::take_shared_from(&cache).is_empty());
        let _ = std::fs::remove_dir_all(cache);
    }
}
