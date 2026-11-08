"""Native acceptance: launch the built application, let its real interface run
inspect -> clean -> verify over IPC on a genuinely signed C2PA sample, and
require a passing result."""
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
app = pathlib.Path(sys.argv[1]).resolve()
if not app.is_file():
    raise SystemExit(f"Application missing: {app}")
with tempfile.TemporaryDirectory(prefix="clear-selftest-") as tmp:
    shutil.copyfile(ROOT / "crates/clear-core/tests/fixtures/CA.jpg", pathlib.Path(tmp) / "sample.jpg")
    env = dict(os.environ, NEXONIC_CLEAR_SELFTEST_DIR=tmp)
    try:
        code = subprocess.run([str(app)], env=env, timeout=120).returncode
    except subprocess.TimeoutExpired:
        raise SystemExit("Native self-test timed out")
    result = pathlib.Path(tmp) / "result.json"
    data = json.loads(result.read_text()) if result.is_file() else {}
    print("Native self-test:", data or "no result", "exit", code)
    if code != 0 or data.get("passed") is not True:
        raise SystemExit(1)
