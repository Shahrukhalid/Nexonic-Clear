"""Collect the Linux AppImage, deb and command-line tool after smoke tests."""
import hashlib
import io
import json
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
out = ROOT / "release-assets"
out.mkdir(exist_ok=True)
build = ROOT / "target/release"
label = "Linux-x64"
version = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())["version"]
images = sorted((build / "bundle/appimage").glob("*.AppImage"))
debs = sorted((build / "bundle/deb").glob("*.deb"))
if len(images) != 1 or len(debs) != 1:
    raise SystemExit(f"expected one AppImage and one deb, found {images} {debs}")
# The deb must contain the app and its notices.
# dpkg-deb lists paths as "./usr/..." or "usr/..." depending on its version.
listing = subprocess.run(["dpkg-deb", "-c", str(debs[0])], check=True, capture_output=True, text=True).stdout
paths = {line.split(maxsplit=5)[-1].lstrip("./") for line in listing.splitlines() if line.strip()}
if "usr/bin/scruvix" not in paths or not any(p.endswith("/notices/INDEX.json") for p in paths):
    raise SystemExit(f"deb is missing the app or its notices: {sorted(paths)[:20]}")
# File managers must be able to open files with the app.
desktop = subprocess.run(["dpkg-deb", "--fsys-tarfile", str(debs[0])], check=True, capture_output=True).stdout
with tarfile.open(fileobj=io.BytesIO(desktop)) as tar:
    entry = next(m for m in tar.getmembers() if m.name.endswith(".desktop") and "applications" in m.name)
    text = tar.extractfile(entry).read().decode()
if "%F" not in text or "MimeType=" not in text or "video/mp4" not in text:
    raise SystemExit(f"desktop entry cannot open files:\n{text}")
shutil.copy2(images[0], out / f"Scruvix-{label}.AppImage")
shutil.copy2(debs[0], out / f"Scruvix-{label}.deb")
cli = build / "scruvix-cli"
subprocess.run([str(cli), "inspect", "-q", "--json", str(ROOT / "crates/clear-core/tests/fixtures/media.pdf")], check=True, stdout=subprocess.DEVNULL)
with tempfile.TemporaryDirectory() as tmp:
    folder = Path(tmp) / "scruvix-cli"
    folder.mkdir()
    shutil.copy2(cli, folder / cli.name)
    shutil.copytree(ROOT / "src-tauri/notices", folder / "notices")
    (folder / "README.txt").write_text(f"Scruvix {version} - {label} command-line tool\n\nRun ./scruvix-cli --help for usage.\n")
    shutil.make_archive(str(out / f"Scruvix-CLI-{label}"), "zip", tmp)
assets = [{"name": p.name, "bytes": p.stat().st_size, "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
          for p in sorted(out.iterdir()) if label in p.name and p.suffix in {".zip", ".AppImage", ".deb"}]
(out / f"VALIDATION-{label}.json").write_text(json.dumps({"platform": label, "assets": assets}, indent=2) + "\n")
for a in assets:
    print(f"{a['name']}: {a['bytes'] / 1048576:.2f} MB")
