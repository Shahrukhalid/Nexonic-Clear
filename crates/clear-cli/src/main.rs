//! `nexonic-clear-cli`: inspect and clean files from a terminal, a script, a
//! file-manager menu or a watched folder. Every result is verified before it
//! is written; unverified results are never written.

mod integrate;

use clear_core::{CleanOptions, CleanReport, Format, Inspection};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

const USAGE: &str = "Nexonic Clear command-line tool

Usage:
  nexonic-clear-cli inspect [options] <files or folders>...
  nexonic-clear-cli clean   [options] <files or folders>...
  nexonic-clear-cli watch   <folder> --out <folder> [options]
  nexonic-clear-cli integrate install|uninstall [--dry-run]

Options:
  -r, --recursive      Include files in subfolders
  -o, --out <dir>      Write cleaned files into this folder
      --in-place       Replace the originals (only after verification passes)
      --suffix <text>  Added before the extension of cleaned copies (default .clean)
      --overwrite      Replace existing output files
      --remove <list>  Also remove these kinds
      --keep <list>    Keep these kinds
                       Kinds: exif,xmp,iptc,icc,c2pa,comments,other,embedded
      --keep-dates     Keep capture dates when EXIF is removed
      --no-orientation Do not keep the orientation tag
      --no-resolution  Do not keep the print resolution
      --json           Print a JSON report on standard output
  -j, --jobs <n>       Files processed in parallel (default: number of CPUs)
  -q, --quiet          No progress messages
      --pause          Wait for Enter before exiting (used by file-manager menus)
      --interval <s>   watch: seconds between scans (default 2)
      --once           watch: process the folder once and exit

Formats: JPEG, PNG, WebP, HEIC, AVIF, WAV, MP3, MP4, MOV, M4A, PDF.
Exit codes: 0 success, 1 a result failed verification, 2 usage error,
3 a file could not be read or is unsupported, 130 cancelled.";

#[derive(Default)]
struct Args {
    command: String,
    paths: Vec<PathBuf>,
    recursive: bool,
    out: Option<PathBuf>,
    in_place: bool,
    suffix: String,
    overwrite: bool,
    json: bool,
    jobs: usize,
    quiet: bool,
    pause: bool,
    interval: u64,
    once: bool,
    dry_run: bool,
    options: CleanOptions,
}

fn kinds(list: &str, o: &mut CleanOptions, value: bool) -> Result<(), String> {
    for k in list.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        match k {
            "exif" => o.exif = value,
            "xmp" => o.xmp = value,
            "iptc" => o.iptc = value,
            "icc" => o.icc = value,
            "c2pa" => o.c2pa = value,
            "comments" | "comment" => o.comments = value,
            "other" => o.other = value,
            "embedded" => o.embedded = value,
            _ => return Err(format!("unknown kind: {k}")),
        }
    }
    Ok(())
}

fn parse_args(raw: Vec<String>) -> Result<Args, String> {
    let mut a = Args {
        suffix: ".clean".into(),
        interval: 2,
        jobs: std::thread::available_parallelism().map_or(2, |n| n.get()),
        ..Default::default()
    };
    let mut it = raw.into_iter();
    a.command = it.next().ok_or("missing command")?;
    if matches!(a.command.as_str(), "-h" | "--help") {
        return Err(String::new());
    }
    let value = |it: &mut std::vec::IntoIter<String>, flag: &str| {
        it.next().ok_or(format!("{flag} needs a value"))
    };
    while let Some(x) = it.next() {
        match x.as_str() {
            "-r" | "--recursive" => a.recursive = true,
            "-o" | "--out" => a.out = Some(PathBuf::from(value(&mut it, &x)?)),
            "--in-place" => a.in_place = true,
            "--suffix" => a.suffix = value(&mut it, &x)?,
            "--overwrite" => a.overwrite = true,
            "--remove" => kinds(&value(&mut it, &x)?, &mut a.options, true)?,
            "--keep" => kinds(&value(&mut it, &x)?, &mut a.options, false)?,
            "--keep-dates" => a.options.keep_dates = true,
            "--no-orientation" => a.options.keep_orientation = false,
            "--no-resolution" => a.options.keep_resolution = false,
            "--json" => a.json = true,
            "-j" | "--jobs" => {
                a.jobs = value(&mut it, &x)?
                    .parse()
                    .map_err(|_| "--jobs needs a number")?;
                a.jobs = a.jobs.max(1);
            }
            "-q" | "--quiet" => a.quiet = true,
            "--pause" => a.pause = true,
            "--interval" => {
                a.interval = value(&mut it, &x)?
                    .parse()
                    .map_err(|_| "--interval needs a number")?
            }
            "--once" => a.once = true,
            "--dry-run" => a.dry_run = true,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("unknown option: {s}")),
            _ => a.paths.push(PathBuf::from(x)),
        }
    }
    if a.in_place && a.out.is_some() {
        return Err("--in-place and --out can't be used together".into());
    }
    if a.suffix.contains(['/', '\\']) {
        return Err("--suffix can't contain a path separator".into());
    }
    Ok(a)
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(raw) {
        Ok(a) => a,
        Err(e) => {
            if e.is_empty() {
                println!("{USAGE}");
                std::process::exit(0);
            }
            eprintln!("error: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let code = match args.command.as_str() {
        "inspect" | "clean" => run_files(&args),
        "watch" => watch(&args),
        "integrate" => integrate::run(&args.paths, args.dry_run),
        "help" => {
            println!("{USAGE}");
            0
        }
        c => {
            eprintln!("error: unknown command: {c}\n\n{USAGE}");
            2
        }
    };
    if args.pause {
        eprintln!("\nPress Enter to close.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    std::process::exit(code);
}

static CANCEL: AtomicBool = AtomicBool::new(false);

fn install_cancel() {
    let _ = ctrlc::set_handler(|| {
        if CANCEL.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
        eprintln!("\nCancelling after the files in progress...");
    });
}

fn supported(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| Format::EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// (file, folder it was found under) for every input. Symbolic links inside
/// folders are not followed.
fn collect(args: &Args) -> Result<Vec<(PathBuf, Option<PathBuf>)>, String> {
    let mut out = Vec::new();
    for p in &args.paths {
        let meta = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
        if meta.is_file() {
            out.push((p.clone(), None));
            continue;
        }
        let mut stack = vec![p.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut entries: Vec<_> = rd.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let Ok(ft) = e.file_type() else { continue };
                let path = e.path();
                if ft.is_dir() && args.recursive {
                    stack.push(path);
                } else if ft.is_file() && supported(&path) && !is_output(&path, &args.suffix) {
                    out.push((path, Some(p.clone())));
                }
            }
        }
    }
    Ok(out)
}

fn is_output(p: &Path, suffix: &str) -> bool {
    !suffix.is_empty()
        && p.file_stem()
            .is_some_and(|s| s.to_string_lossy().ends_with(suffix))
}

fn output_for(args: &Args, file: &Path, root: Option<&Path>) -> PathBuf {
    if args.in_place {
        return file.to_path_buf();
    }
    let name = file.file_name().unwrap_or_default();
    match &args.out {
        Some(dir) => {
            let rel = root
                .and_then(|r| file.strip_prefix(r).ok())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(name));
            dir.join(rel)
        }
        None => {
            let stem = file.file_stem().unwrap_or_default().to_string_lossy();
            let ext = file
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            file.with_file_name(format!("{stem}{}{ext}", args.suffix))
        }
    }
}

#[derive(Serialize)]
struct ErrorInfo {
    code: String,
    message: String,
}

#[derive(Serialize)]
struct Entry {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inspection: Option<Inspection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    report: Option<CleanReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorInfo>,
}

fn err(e: &clear_core::Error) -> ErrorInfo {
    ErrorInfo {
        code: e.code().into(),
        message: e.to_string(),
    }
}

fn io_err(m: String) -> ErrorInfo {
    ErrorInfo {
        code: "io".into(),
        message: m,
    }
}

fn process(args: &Args, file: &Path, root: Option<&Path>) -> Entry {
    let mut e = Entry {
        path: file.display().to_string(),
        output: None,
        inspection: None,
        report: None,
        error: None,
    };
    if args.command == "inspect" {
        match clear_core::inspect_path(file) {
            Ok(i) => e.inspection = Some(i),
            Err(x) => e.error = Some(err(&x)),
        }
        return e;
    }
    let out = output_for(args, file, root);
    if !args.in_place && !args.overwrite && out.exists() {
        e.error = Some(io_err(format!(
            "{} already exists (use --overwrite)",
            out.display()
        )));
        return e;
    }
    if let Some(parent) = out.parent() {
        if let Err(x) = std::fs::create_dir_all(parent) {
            e.error = Some(io_err(format!("{}: {x}", parent.display())));
            return e;
        }
    }
    match clear_core::clean_path(file, &out, &args.options) {
        Ok(r) => {
            if r.verified {
                e.output = Some(out.display().to_string());
            }
            e.report = Some(r);
        }
        Err(x) => e.error = Some(err(&x)),
    }
    e
}

fn describe(e: &Entry) -> String {
    if let Some(x) = &e.error {
        return format!("error: {}: {}", e.path, x.message);
    }
    if let Some(i) = &e.inspection {
        let mut s = format!(
            "{} ({}, {} bytes)",
            e.path,
            i.format.extension().to_uppercase(),
            i.size
        );
        if i.blocks.is_empty() {
            s.push_str("\n  no removable metadata");
        }
        for b in &i.blocks {
            s.push_str(&format!("\n  {:?}: {} ({} bytes)", b.kind, b.label, b.size));
        }
        if i.credentials.present {
            s.push_str(&format!("\n  Content Credentials: {}", i.credentials.state));
        }
        for w in &i.warnings {
            s.push_str(&format!("\n  note: {w}"));
        }
        return s;
    }
    let r = e.report.as_ref().unwrap();
    if r.verified {
        format!(
            "cleaned: {} -> {} ({} blocks removed, {} bytes smaller)",
            e.path,
            e.output.as_deref().unwrap_or_default(),
            r.before.blocks.len().saturating_sub(r.after.blocks.len()),
            r.bytes_removed
        )
    } else {
        let why: Vec<&str> = r.problems.iter().map(|p| p.text.as_str()).collect();
        format!(
            "not verified, nothing written: {}: {}",
            e.path,
            why.join(" ")
        )
    }
}

fn exit_code(entries: &[Entry]) -> i32 {
    if entries.iter().any(|e| e.error.is_some()) {
        3
    } else if entries
        .iter()
        .any(|e| e.report.as_ref().is_some_and(|r| !r.verified))
    {
        1
    } else {
        0
    }
}

/// Runs files on a pool of workers, in input order for the report.
fn run_batch(args: &Args, files: &[(PathBuf, Option<PathBuf>)]) -> Vec<Option<Entry>> {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Entry>>> = Mutex::new((0..files.len()).map(|_| None).collect());
    let cancel_after: Option<usize> = std::env::var("NEXONIC_TEST_CANCEL_AFTER")
        .ok()
        .and_then(|v| v.parse().ok());
    std::thread::scope(|s| {
        for _ in 0..args.jobs.min(files.len()).max(1) {
            s.spawn(|| loop {
                if CANCEL.load(Ordering::SeqCst) {
                    break;
                }
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some((file, root)) = files.get(i) else {
                    break;
                };
                let e = process(args, file, root.as_deref());
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                if !args.quiet {
                    eprintln!(
                        "[{n}/{}] {}",
                        files.len(),
                        describe(&e).lines().next().unwrap_or_default()
                    );
                }
                results.lock().unwrap()[i] = Some(e);
                if cancel_after.is_some_and(|c| n >= c) {
                    CANCEL.store(true, Ordering::SeqCst);
                }
            });
        }
    });
    results.into_inner().unwrap()
}

fn run_files(args: &Args) -> i32 {
    if args.paths.is_empty() {
        eprintln!("error: no files given\n\n{USAGE}");
        return 2;
    }
    install_cancel();
    let files = match collect(args) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return 3;
        }
    };
    if files.is_empty() {
        eprintln!("No supported files found.");
        return 3;
    }
    let results = run_batch(args, &files);
    let cancelled = results.iter().any(Option::is_none);
    let entries: Vec<Entry> = results.into_iter().flatten().collect();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&entries).unwrap());
    } else if args.command == "inspect" || args.quiet {
        for e in &entries {
            println!("{}", describe(e));
        }
    }
    if !args.quiet {
        let ok = entries
            .iter()
            .filter(|e| e.error.is_none() && e.report.as_ref().is_none_or(|r| r.verified))
            .count();
        eprintln!("{ok} of {} files done.", files.len());
    }
    if cancelled {
        eprintln!(
            "Cancelled: {} of {} files were not processed.",
            files.len() - entries.len(),
            files.len()
        );
        return 130;
    }
    exit_code(&entries)
}

/// Cleans files that appear in a folder once their size and modification time
/// stop changing. Results go to `--out`; unverified results are never written.
fn watch(args: &Args) -> i32 {
    let (Some(dir), Some(out)) = (args.paths.first(), args.out.as_ref()) else {
        eprintln!("error: watch needs a folder and --out <folder>");
        return 2;
    };
    if args.in_place {
        eprintln!("error: watch can't clean in place; use --out");
        return 2;
    }
    let (Ok(d), Ok(_)) = (dir.canonicalize(), std::fs::create_dir_all(out)) else {
        eprintln!(
            "error: can't open {} or create {}",
            dir.display(),
            out.display()
        );
        return 3;
    };
    if out.canonicalize().is_ok_and(|o| o.starts_with(&d)) {
        eprintln!("error: --out must be outside the watched folder");
        return 2;
    }
    install_cancel();
    let mut a = Args {
        command: "clean".into(),
        paths: vec![dir.clone()],
        recursive: args.recursive,
        out: Some(out.clone()),
        suffix: String::new(),
        overwrite: true,
        json: args.json,
        jobs: args.jobs,
        quiet: args.quiet,
        options: args.options.clone(),
        ..Default::default()
    };
    a.interval = args.interval;
    let mut pending: std::collections::HashMap<PathBuf, (u64, Option<SystemTime>)> =
        Default::default();
    let mut handled: std::collections::HashMap<PathBuf, (u64, Option<SystemTime>)> =
        Default::default();
    let mut worst = 0;
    if !args.quiet {
        eprintln!("Watching {} (Ctrl+C to stop)", dir.display());
    }
    loop {
        let files = collect(&a).unwrap_or_default();
        let mut ready = Vec::new();
        for (f, root) in files {
            let Ok(m) = std::fs::metadata(&f) else {
                continue;
            };
            let sig = (m.len(), m.modified().ok());
            if handled.get(&f) == Some(&sig) {
                continue;
            }
            if args.once || pending.get(&f) == Some(&sig) {
                ready.push((f.clone(), root));
                handled.insert(f.clone(), sig);
                pending.remove(&f);
            } else {
                pending.insert(f, sig);
            }
        }
        if !ready.is_empty() {
            let entries: Vec<Entry> = run_batch(&a, &ready).into_iter().flatten().collect();
            if args.json {
                for e in &entries {
                    println!("{}", serde_json::to_string(e).unwrap());
                }
            }
            worst = worst.max(exit_code(&entries));
        }
        if args.once || CANCEL.load(Ordering::SeqCst) {
            return if CANCEL.load(Ordering::SeqCst) {
                130
            } else {
                worst
            };
        }
        let mut slept = 0;
        while slept < a.interval * 10 && !CANCEL.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
            slept += 1;
        }
    }
}
