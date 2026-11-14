//! "Clean with Nexonic Clear" in the file manager's right-click menu, for the
//! current user only (no administrator rights needed):
//! - Windows Explorer: per-extension and folder verbs under HKCU\Software\Classes;
//! - macOS Finder: a Quick Action in ~/Library/Services;
//! - Linux: a Nautilus (GNOME Files) script and a Dolphin (KDE) service menu.
//!
//! The menu runs `nexonic-clear-cli clean --pause` on the selection, which
//! writes verified `.clean` copies next to the originals.

use std::path::{Path, PathBuf};

const LABEL: &str = "Clean with Nexonic Clear";

pub fn run(args: &[PathBuf], dry_run: bool) -> i32 {
    let action = args.first().and_then(|a| a.to_str()).unwrap_or_default();
    let install = match action {
        "install" => true,
        "uninstall" => false,
        _ => {
            eprintln!("error: use `integrate install` or `integrate uninstall`");
            return 2;
        }
    };
    let exe = match std::env::current_exe().and_then(|p| p.canonicalize()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: can't find this program's path: {e}");
            return 3;
        }
    };
    let result = if cfg!(windows) {
        windows(&exe, install, dry_run)
    } else if cfg!(target_os = "macos") {
        macos(&exe, install, dry_run)
    } else {
        linux(&exe, install, dry_run)
    };
    match result {
        Ok(done) => {
            for d in done {
                println!("{d}");
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            3
        }
    }
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".into())
}

fn write(
    path: &Path,
    body: &str,
    executable: bool,
    dry: bool,
    log: &mut Vec<String>,
) -> Result<(), String> {
    log.push(format!("write {}", path.display()));
    if dry {
        return Ok(());
    }
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    std::fs::write(path, body).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    Ok(())
}

fn remove(path: &Path, dry: bool, log: &mut Vec<String>) {
    if path.exists() {
        log.push(format!("remove {}", path.display()));
        if !dry {
            let _ = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
        }
    }
}

fn sh_quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', r"'\''"))
}

pub(crate) fn linux(exe: &Path, install: bool, dry: bool) -> Result<Vec<String>, String> {
    let home = home()?;
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let script = data.join("nautilus/scripts").join(LABEL);
    let dolphin = data.join("kio/servicemenus/nexonic-clear.desktop");
    let mut log = Vec::new();
    if !install {
        remove(&script, dry, &mut log);
        remove(&dolphin, dry, &mut log);
        return Ok(log);
    }
    let terminal = "x-terminal-emulator";
    let body = format!(
        "#!/bin/sh\n# {LABEL}: writes verified .clean copies next to the selected files.\n\
         IFS='\n'\nset -- $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\n\
         if [ -n \"$DISPLAY$WAYLAND_DISPLAY\" ] && command -v {terminal} >/dev/null 2>&1; then\n  exec {terminal} -e {exe} clean -r --pause \"$@\"\nfi\n\
         exec {exe} clean -r -q \"$@\"\n",
        exe = sh_quote(exe)
    );
    write(&script, &body, true, dry, &mut log)?;
    let mimes = "image/jpeg;image/png;image/webp;image/heic;image/heif;image/avif;audio/x-wav;audio/wav;audio/mpeg;audio/mp4;video/mp4;video/quicktime;application/pdf;inode/directory;";
    let desktop = format!(
        "[Desktop Entry]\nType=Service\nMimeType={mimes}\nActions=clean;\nX-KDE-Priority=TopLevel\n\n\
         [Desktop Action clean]\nName={LABEL}\nIcon=edit-clear\nExec=konsole --hold -e {exe} clean -r %F\n",
        exe = sh_quote(exe)
    );
    write(&dolphin, &desktop, true, dry, &mut log)?;
    Ok(log)
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn macos(exe: &Path, install: bool, dry: bool) -> Result<Vec<String>, String> {
    let dir = home()?
        .join("Library/Services")
        .join(format!("{LABEL}.workflow"));
    let mut log = Vec::new();
    if !install {
        remove(&dir, dry, &mut log);
        return Ok(log);
    }
    let info = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>NSServices</key>
	<array>
		<dict>
			<key>NSMenuItem</key>
			<dict>
				<key>default</key>
				<string>{LABEL}</string>
			</dict>
			<key>NSMessage</key>
			<string>runWorkflowAsService</string>
			<key>NSRequiredContext</key>
			<dict>
				<key>NSApplicationIdentifier</key>
				<string>com.apple.finder</string>
			</dict>
			<key>NSSendFileTypes</key>
			<array>
				<string>public.item</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
"#
    );
    let command = xml(&format!(
        "{} clean -r -q \"$@\" > \"$TMPDIR/nexonic-clear-last.txt\" 2>&1\nosascript -e 'display notification \"Verified .clean copies were saved next to the originals.\" with title \"Nexonic Clear\"'",
        sh_quote(exe)
    ));
    let wflow = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>523</string>
	<key>AMApplicationVersion</key>
	<string>2.10</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMParameterProperties</key>
				<dict>
					<key>COMMAND_STRING</key>
					<dict/>
					<key>CheckedForUserDefaultShell</key>
					<dict/>
					<key>inputMethod</key>
					<dict/>
					<key>shell</key>
					<dict/>
					<key>source</key>
					<dict/>
				</dict>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>{command}</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/bash</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>6A1C8C2E-6D3B-4E7A-9F2D-1B7E0C4A5D10</string>
				<key>Keywords</key>
				<array>
					<string>Shell</string>
					<string>Script</string>
				</array>
				<key>OutputUUID</key>
				<string>0E3F7B21-58C4-4C59-8E36-2F9A1D6B7C20</string>
				<key>UUID</key>
				<string>B5D3A9F0-2C71-4F8E-A6D4-93E1C7B2F830</string>
				<key>UnlocalizedApplications</key>
				<array>
					<string>Automator</string>
				</array>
				<key>arguments</key>
				<dict/>
				<key>isViewVisible</key>
				<integer>1</integer>
			</dict>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>applicationBundleIDsByPath</key>
		<dict/>
		<key>applicationPaths</key>
		<array/>
		<key>inputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject</string>
		<key>outputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>presentationMode</key>
		<integer>15</integer>
		<key>processesInput</key>
		<false/>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<false/>
		<key>systemImageName</key>
		<string>NSActionTemplate</string>
		<key>useAutomaticInputType</key>
		<false/>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>
"#
    );
    write(
        &dir.join("Contents/Info.plist"),
        &info,
        false,
        dry,
        &mut log,
    )?;
    write(
        &dir.join("Contents/document.wflow"),
        &wflow,
        false,
        dry,
        &mut log,
    )?;
    if !dry && cfg!(target_os = "macos") {
        let _ = std::process::Command::new("/System/Library/CoreServices/pbs")
            .arg("-update")
            .status();
    }
    Ok(log)
}

/// `reg.exe` commands for the current user's Explorer menu.
pub(crate) fn windows_commands(exe: &Path, install: bool) -> Vec<Vec<String>> {
    let mut keys: Vec<(String, &str)> = clear_core::Format::EXTENSIONS
        .iter()
        .map(|e| {
            (
                format!(r"HKCU\Software\Classes\SystemFileAssociations\.{e}\shell\NexonicClear"),
                "\"%1\"",
            )
        })
        .collect();
    keys.push((
        r"HKCU\Software\Classes\Directory\shell\NexonicClear".into(),
        "-r \"%1\"",
    ));
    let mut cmds = Vec::new();
    for (key, arg) in keys {
        if install {
            cmds.push(vec![
                "add".into(),
                key.clone(),
                "/ve".into(),
                "/d".into(),
                LABEL.into(),
                "/f".into(),
            ]);
            cmds.push(vec![
                "add".into(),
                key.clone(),
                "/v".into(),
                "Icon".into(),
                "/d".into(),
                exe.display().to_string(),
                "/f".into(),
            ]);
            cmds.push(vec![
                "add".into(),
                format!(r"{key}\command"),
                "/ve".into(),
                "/d".into(),
                format!("\"{}\" clean --pause {arg}", exe.display()),
                "/f".into(),
            ]);
        } else {
            cmds.push(vec!["delete".into(), key, "/f".into()]);
        }
    }
    cmds
}

fn windows(exe: &Path, install: bool, dry: bool) -> Result<Vec<String>, String> {
    let mut log = Vec::new();
    for c in windows_commands(exe, install) {
        log.push(format!("reg {}", c.join(" ")));
        if dry {
            continue;
        }
        let ok = std::process::Command::new("reg")
            .args(&c)
            .output()
            .map_err(|e| format!("reg: {e}"))?;
        if !ok.status.success() && install {
            return Err(format!(
                "reg {} failed: {}",
                c.join(" "),
                String::from_utf8_lossy(&ok.stderr)
            ));
        }
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_commands_cover_every_format_and_folders() {
        let exe = Path::new(r"C:\Program Files\Nexonic Clear\nexonic-clear-cli.exe");
        let add = windows_commands(exe, true);
        for e in clear_core::Format::EXTENSIONS {
            assert!(
                add.iter().any(|c| c[1].contains(&format!(r"\.{e}\shell"))),
                "{e}"
            );
        }
        let cmd = add
            .iter()
            .find(|c| c[1].ends_with(r"Directory\shell\NexonicClear\command"))
            .unwrap();
        assert_eq!(
            cmd[4],
            format!("\"{}\" clean --pause -r \"%1\"", exe.display())
        );
        assert!(windows_commands(exe, false)
            .iter()
            .all(|c| c[0] == "delete"));
    }

    #[test]
    fn macos_quick_action_is_a_valid_plist_with_the_command() {
        let tmp = std::env::temp_dir().join(format!("nexonic-mac-{}", std::process::id()));
        std::env::set_var("HOME", &tmp);
        let exe = Path::new("/Applications/Nexonic Clear.app/Contents/MacOS/nexonic-clear-cli");
        macos(exe, true, false).unwrap();
        let wf = tmp.join("Library/Services/Clean with Nexonic Clear.workflow/Contents");
        let doc = std::fs::read_to_string(wf.join("document.wflow")).unwrap();
        assert!(doc.contains("&apos;/Applications/Nexonic Clear.app") || doc.contains("'/Applications/Nexonic Clear.app/Contents/MacOS/nexonic-clear-cli' clean -r -q"));
        assert!(std::fs::read_to_string(wf.join("Info.plist"))
            .unwrap()
            .contains("runWorkflowAsService"));
        macos(exe, false, false).unwrap();
        assert!(!wf.exists());
        let _ = std::fs::remove_dir_all(tmp);
    }
}
