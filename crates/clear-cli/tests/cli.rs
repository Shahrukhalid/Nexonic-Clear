//! Runs the real command-line tool on the real fixtures.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_nexonic-clear-cli");

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../clear-core/tests/fixtures")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("nexonic-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A folder with every media fixture plus a JPEG, one level nested.
fn sample_tree(name: &str) -> PathBuf {
    let d = tmp(name);
    std::fs::create_dir_all(d.join("sub")).unwrap();
    for f in std::fs::read_dir(fixtures()).unwrap().flatten() {
        let n = f.file_name().to_string_lossy().to_string();
        if n.starts_with("media.") {
            std::fs::copy(f.path(), d.join(&n)).unwrap();
        }
    }
    std::fs::copy(fixtures().join("IMG_0003.jpg"), d.join("sub/photo.jpg")).unwrap();
    std::fs::write(d.join("notes.txt"), "not media").unwrap();
    d
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().unwrap()
}

fn json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)))
}

#[test]
fn inspect_reports_every_file_as_json() {
    let d = sample_tree("inspect");
    let o = run(&["inspect", "-r", "--json", "-q", d.to_str().unwrap()]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v = json(&o);
    let list = v.as_array().unwrap();
    assert_eq!(list.len(), 9, "8 media fixtures and 1 photo");
    for e in list {
        assert!(
            e["inspection"]["blocks"]
                .as_array()
                .is_some_and(|b| !b.is_empty()),
            "{e}"
        );
    }
}

#[test]
fn clean_writes_verified_copies_in_parallel_and_mirrors_folders() {
    let d = sample_tree("clean");
    let out = tmp("clean-out");
    let o = run(&[
        "clean",
        "-r",
        "-j",
        "4",
        "--json",
        "--remove",
        "c2pa,embedded,icc",
        "--no-resolution",
        "--no-orientation",
        "-o",
        out.to_str().unwrap(),
        d.to_str().unwrap(),
    ]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let v = json(&o);
    for e in v.as_array().unwrap() {
        assert_eq!(e["report"]["verified"], true, "{}", e["path"]);
        assert!(
            e["report"]["after"]["blocks"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{}",
            e["path"]
        );
    }
    assert!(out.join("sub/photo.jpg").is_file());
    for f in ["media.mp4", "media.pdf", "media.heic", "media.wav"] {
        let cleaned = std::fs::read(out.join(f)).unwrap();
        assert!(!cleaned.windows(6).any(|w| w == b"Secret"), "{f}");
    }
    // Progress goes to stderr, one line per file.
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("[9/9]"), "{err}");
}

#[test]
fn default_output_is_next_to_the_original_and_never_overwrites() {
    let d = tmp("suffix");
    let src = d.join("clip.mp3");
    std::fs::copy(fixtures().join("media.mp3"), &src).unwrap();
    let o = run(&["clean", "-q", src.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    assert!(d.join("clip.clean.mp3").is_file());
    let again = run(&["clean", "-q", src.to_str().unwrap()]);
    assert_eq!(
        again.status.code(),
        Some(3),
        "existing output must not be replaced"
    );
    // A folder run skips earlier outputs.
    let o = run(&["inspect", "-q", "--json", d.to_str().unwrap()]);
    assert_eq!(json(&o).as_array().unwrap().len(), 1);
}

#[test]
fn in_place_replaces_only_after_verification() {
    let d = tmp("inplace");
    let f = d.join("doc.pdf");
    std::fs::copy(fixtures().join("media.pdf"), &f).unwrap();
    let before = std::fs::read(&f).unwrap();
    let o = run(&["clean", "-q", "--in-place", f.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0));
    let after = std::fs::read(&f).unwrap();
    assert!(after.len() < before.len());
    assert_eq!(
        std::fs::read_dir(&d).unwrap().count(),
        1,
        "no temporary files left"
    );
}

#[test]
fn errors_and_usage_have_distinct_exit_codes() {
    let d = tmp("errors");
    let bad = d.join("fake.jpg");
    std::fs::write(&bad, b"not a jpeg at all").unwrap();
    assert_eq!(
        run(&["clean", "-q", bad.to_str().unwrap()]).status.code(),
        Some(3)
    );
    assert_eq!(run(&["clean", "--bogus"]).status.code(), Some(2));
    assert_eq!(run(&["frobnicate"]).status.code(), Some(2));
    assert_eq!(run(&["--help"]).status.code(), Some(0));
}

#[test]
fn cancelling_stops_before_the_remaining_files() {
    let d = sample_tree("cancel");
    let out = tmp("cancel-out");
    let o = Command::new(BIN)
        .args([
            "clean",
            "-r",
            "-j",
            "1",
            "-o",
            out.to_str().unwrap(),
            d.to_str().unwrap(),
        ])
        .env("NEXONIC_TEST_CANCEL_AFTER", "2")
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let written = walk(&out);
    assert_eq!(written, 2, "only the files finished before cancelling");
}

fn walk(d: &Path) -> usize {
    std::fs::read_dir(d)
        .unwrap()
        .flatten()
        .map(|e| {
            if e.path().is_dir() {
                walk(&e.path())
            } else {
                1
            }
        })
        .sum()
}

#[test]
fn watch_cleans_new_files_into_the_output_folder() {
    let d = tmp("watch-in");
    let out = tmp("watch-out");
    std::fs::copy(fixtures().join("media.m4a"), d.join("song.m4a")).unwrap();
    let o = run(&[
        "watch",
        d.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--once",
        "--json",
        "-q",
    ]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let line: serde_json::Value =
        serde_json::from_slice(o.stdout.split(|&b| b == b'\n').next().unwrap()).unwrap();
    assert_eq!(line["report"]["verified"], true);
    assert!(out.join("song.m4a").is_file());
    // Output inside the watched folder is refused (it would loop).
    let inner = d.join("out");
    assert_eq!(
        run(&[
            "watch",
            d.to_str().unwrap(),
            "-o",
            inner.to_str().unwrap(),
            "--once"
        ])
        .status
        .code(),
        Some(2)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn integrate_installs_and_removes_file_manager_menus() {
    let home = tmp("home");
    let o = Command::new(BIN)
        .args(["integrate", "install"])
        .env("HOME", &home)
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let script = home.join(".local/share/nautilus/scripts/Clean with Nexonic Clear");
    let menu = home.join(".local/share/kio/servicemenus/nexonic-clear.desktop");
    let body = std::fs::read_to_string(&script).unwrap();
    assert!(body.contains("nexonic-clear-cli' clean -r"));
    assert!(std::fs::read_to_string(&menu)
        .unwrap()
        .contains("MimeType=image/jpeg;"));
    // The script really cleans what Nautilus passes in.
    let f = tmp("nautilus").join("a.wav");
    std::fs::copy(fixtures().join("media.wav"), &f).unwrap();
    let status = Command::new("sh")
        .arg(&script)
        .env(
            "NAUTILUS_SCRIPT_SELECTED_FILE_PATHS",
            format!("{}\n", f.display()),
        )
        .env("PATH", "/usr/bin:/bin")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(status.status.success() || f.with_file_name("a.clean.wav").is_file());
    assert!(
        f.with_file_name("a.clean.wav").is_file(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let o = Command::new(BIN)
        .args(["integrate", "uninstall"])
        .env("HOME", &home)
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    assert!(!script.exists() && !menu.exists());
}
