"""Sign release APKs with the CI development certificate and verify signature,
ABI contents, notices and 16 KiB native alignment."""
import hashlib
import json
import os
import shutil
import struct
import subprocess
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
proof = ROOT / "test-results/android-selftest.json"
if json.loads(proof.read_text()).get("passed") is not True:
    raise SystemExit("Android device self-test proof required")
sdk = Path(os.environ["ANDROID_HOME"])
tools = sorted((sdk / "build-tools").iterdir(), key=lambda p: tuple(int(x) for x in p.name.split(".") if x.isdigit()))[-1]
# Development signing key generated for this build (not a store or production key).
keystore = ROOT / "test-results/development-signing.jks"
if not keystore.exists():
    subprocess.run(["keytool", "-genkeypair", "-noprompt", "-keystore", keystore, "-alias", "nexonicdev",
                    "-keyalg", "RSA", "-keysize", "3072", "-validity", "3650", "-storepass", "android",
                    "-keypass", "android", "-dname", "CN=Nexonic Clear Development, O=Nexonic Technologies"], check=True)
version = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())["version"]
out = ROOT / "release-assets"
out.mkdir(exist_ok=True)
apks = ROOT / "src-tauri/gen/android/app/build/outputs/apk"
records = []
for abi, label, variant in [("arm64-v8a", "ARM64", "arm64"), ("armeabi-v7a", "ARM32", "arm")]:
    found = list(apks.glob(f"{variant}/release/*.apk"))
    if len(found) != 1:
        raise SystemExit(f"Expected one {abi} release APK, found {found}")
    name = f"NexonicClear-Android-{label}.apk"
    aligned = out / name
    subprocess.run([tools / "zipalign", "-f", "-P", "16", "4", found[0], aligned], check=True)
    subprocess.run([tools / "apksigner", "sign", "--ks", keystore, "--ks-pass", "pass:android",
                    "--ks-key-alias", "nexonicdev", "--key-pass", "pass:android", aligned], check=True)
    subprocess.run([tools / "apksigner", "verify", aligned], check=True)
    subprocess.run([tools / "zipalign", "-c", "-P", "16", "4", aligned], check=True)
    with zipfile.ZipFile(aligned) as archive:
        names = archive.namelist()
        libs = [n for n in names if n.startswith("lib/") and n.endswith(".so")]
        if not libs or any(n.split("/")[1] != abi for n in libs):
            raise SystemExit("APK ABI mismatch")
        if "assets/notices/INDEX.json" not in names:
            raise SystemExit("Android notices missing")
        for lib in libs:
            elf = archive.read(lib)
            is64 = elf[4] == 2
            off = struct.unpack_from("<Q" if is64 else "<I", elf, 32 if is64 else 28)[0]
            size, count = struct.unpack_from("<HH", elf, 54 if is64 else 42)
            for i in range(count):
                pos = off + size * i
                if struct.unpack_from("<I", elf, pos)[0] == 1:
                    align = struct.unpack_from("<Q" if is64 else "<I", elf, pos + (48 if is64 else 28))[0]
                    if align < 16384:
                        raise SystemExit(f"{lib}: native page alignment below 16 KiB")
    records.append({"name": name, "bytes": aligned.stat().st_size, "sha256": hashlib.sha256(aligned.read_bytes()).hexdigest()})
    print(f"{name}: {aligned.stat().st_size / 1048576:.2f} MB")
(out / "VALIDATION-Android.json").write_text(json.dumps(
    {"platform": "Android", "deviceSelfTest": "passed", "signing": "development certificate", "assets": records}, indent=2) + "\n")
