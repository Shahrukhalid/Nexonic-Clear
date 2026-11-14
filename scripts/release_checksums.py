"""Refuse to publish unless every platform passed and the asset set is complete."""
import hashlib
import json
from pathlib import Path

out = Path("release-assets")
SUFFIXES = {".zip", ".apk", ".aab", ".AppImage", ".deb"}
records = []
for label in ["Windows-x64", "macOS-AppleSilicon", "macOS-Intel", "Linux-x64", "Android"]:
    records += json.loads((out / f"VALIDATION-{label}.json").read_text())["assets"]
payload = sorted(p.name for p in out.iterdir() if p.suffix in SUFFIXES)
names = sorted(r["name"] for r in records)
if names != payload or len(payload) < 9:
    raise SystemExit(f"Unexpected release payload: {payload} (validated: {names})")
for r in records:
    p = out / r["name"]
    if p.stat().st_size != r["bytes"] or hashlib.sha256(p.read_bytes()).hexdigest() != r["sha256"]:
        raise SystemExit(f"Integrity check failed: {p.name}")
(out / "SHA256SUMS.txt").write_text("".join(f"{r['sha256']}  {r['name']}\n" for r in sorted(records, key=lambda r: r["name"])))
print("Release set complete and verified.")
