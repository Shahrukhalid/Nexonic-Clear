"""Run native acceptance on the release build, then collect portable packages."""
import hashlib
import json
import platform
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
    "Nexonic Clear {version} - {label}\n\n"
    "Portable: no installer and no setup. Works fully offline.\n"
    "{run}\n\n"
    "Third-party licenses are in the notices folder.\n"
    "https://github.com/Shahrukhalid/Nexonic-Clear\n"
)
version = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())["version"]


def selftest(executable):
    subprocess.run([sys.executable, str(ROOT / "scripts/selftest.py"), str(executable)], check=True)


if platform.system() == "Windows":
    label = "Windows-x64"
    exe = build / "nexonic-clear.exe"
    selftest(exe)
    with tempfile.TemporaryDirectory() as tmp:
        folder = Path(tmp) / "NexonicClear"
        folder.mkdir()
        shutil.copyfile(exe, folder / "NexonicClear.exe")
        shutil.copytree(notices, folder / "notices")
        (folder / "README.txt").write_text(readme.format(version=version, label=label,
            run="Extract the folder, then open NexonicClear.exe.\nRequires the Microsoft Edge WebView2 Runtime, included with Windows 10 and 11.\nThis build is not code-signed, so SmartScreen may ask for confirmation."))
        selftest(folder / "NexonicClear.exe")
        shutil.make_archive(str(out / f"NexonicClear-{label}-Portable"), "zip", tmp)
else:
    label = "macOS-" + ("AppleSilicon" if platform.machine() == "arm64" else "Intel")
    app = build / "bundle/macos/Nexonic Clear.app"
    if not (app / "Contents/Resources/notices/INDEX.json").is_file():
        raise SystemExit("App bundle is missing its notices")
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
    selftest(app / "Contents/MacOS/nexonic-clear")
    subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app),
                    str(out / f"NexonicClear-{label}.zip")], check=True)

assets = [{"name": p.name, "bytes": p.stat().st_size, "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
          for p in sorted(out.glob(f"*{label}*.zip"))]
(out / f"VALIDATION-{label}.json").write_text(json.dumps(
    {"platform": label, "nativeSelfTest": "passed", "assets": assets}, indent=2) + "\n")
for a in assets:
    print(f"{a['name']}: {a['bytes'] / 1048576:.2f} MB")
