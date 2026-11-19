"""Run native acceptance on the release build, then collect portable packages."""
import hashlib
import json
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
out = ROOT / "release-assets"
out.mkdir(exist_ok=True)
notices = ROOT / "src-tauri/notices"
if not (notices / "INDEX.json").is_file():
    raise SystemExit("Collect dependency notices first")
build = ROOT / "target/release"
readme = (
    "Scruvix {version} - {label}\n\n"
    "Portable: no installer and no setup. Works fully offline.\n"
    "{run}\n\n"
    "Third-party licenses are in the notices folder.\n"
    "https://github.com/nexonic-technologies/Scruvix\n"
)
config = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())
version = config["version"]


def selftest(executable):
    subprocess.run([sys.executable, str(ROOT / "scripts/selftest.py"), str(executable)], check=True)


if platform.system() == "Windows":
    label = "Windows-x64"
    exe = build / "scruvix.exe"
    selftest(exe)
    with tempfile.TemporaryDirectory() as tmp:
        folder = Path(tmp) / "Scruvix"
        folder.mkdir()
        shutil.copyfile(exe, folder / "Scruvix.exe")
        shutil.copytree(notices, folder / "notices")
        (folder / "README.txt").write_text(readme.format(version=version, label=label,
            run="Extract the folder, then open Scruvix.exe.\nRequires the Microsoft Edge WebView2 Runtime, included with Windows 10 and 11.\nSmartScreen may ask for confirmation when publisher signing is unavailable."))
        selftest(folder / "Scruvix.exe")
        shutil.make_archive(str(out / f"Scruvix-{label}-Portable"), "zip", tmp)
else:
    label = "macOS-" + ("AppleSilicon" if platform.machine() == "arm64" else "Intel")
    app = build / "bundle/macos/Scruvix.app"
    if not (app / "Contents/Resources/notices/INDEX.json").is_file():
        raise SystemExit("App bundle is missing its notices")
    with (app / "Contents/Info.plist").open("rb") as plist:
        info = plistlib.load(plist)
    if info.get("CFBundleIdentifier") != config["identifier"] or info.get("CFBundleShortVersionString") != version:
        raise SystemExit("Unexpected macOS bundle identifier or version")
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
    selftest(app / "Contents/MacOS/scruvix")
    subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app),
                    str(out / f"Scruvix-{label}.zip")], check=True)

# Command-line tool: smoke-test it on a real file, then ship it as its own zip.
cli = build / ("scruvix-cli.exe" if platform.system() == "Windows" else "scruvix-cli")
if cli.is_file():
    sample = ROOT / "crates/clear-core/tests/fixtures/media.mp4"
    subprocess.run([str(cli), "inspect", "--json", "-q", str(sample)], check=True, stdout=subprocess.DEVNULL)
    with tempfile.TemporaryDirectory() as tmp:
        folder = Path(tmp) / "scruvix-cli"
        folder.mkdir()
        shutil.copy2(cli, folder / cli.name)
        shutil.copytree(notices, folder / "notices")
        (folder / "README.txt").write_text(readme.format(version=version, label=label + " command-line tool",
            run=f"Run {cli.name} --help for usage. `{cli.name} integrate install` adds \"Clean with Scruvix\" to the file manager menu."))
        shutil.make_archive(str(out / f"Scruvix-CLI-{label}"), "zip", tmp)

assets = [{"name": p.name, "bytes": p.stat().st_size, "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
          for p in sorted(out.glob(f"*{label}*.zip"))]
(out / f"VALIDATION-{label}.json").write_text(json.dumps(
    {"platform": label, "nativeSelfTest": "passed", "assets": assets}, indent=2) + "\n")
for a in assets:
    print(f"{a['name']}: {a['bytes'] / 1048576:.2f} MB")
