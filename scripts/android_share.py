"""Adds the Android share sheet to the generated Android project.

Run after `tauri android init`. Registers the main activity for SEND,
SEND_MULTIPLE and VIEW of images, audio, video and PDFs, and copies the
shared content into `<cache>/shared`, where the app's `take_shared` command
picks it up. Running it twice changes nothing.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
APP = ROOT / "src-tauri/gen/android/app/src/main"
MIMES = ["image/*", "audio/*", "video/*", "application/pdf"]
MARK = "<!-- nexonic-share -->"


def manifest(path: Path) -> None:
    text = path.read_text()
    if MARK in text:
        return
    data = "".join(f'<data android:mimeType="{m}" />' for m in MIMES)
    filters = MARK + "".join(
        f'<intent-filter><action android:name="android.intent.action.{a}" />'
        f'<category android:name="android.intent.category.DEFAULT" />{data}</intent-filter>'
        for a in ("SEND", "SEND_MULTIPLE", "VIEW")
    )
    # Insert into the activity that has the launcher intent filter.
    m = re.search(r"<activity\b[^>]*>(?:(?!</activity>).)*?android\.intent\.category\.LAUNCHER(?:(?!</activity>).)*?</activity>", text, re.S)
    if not m:
        sys.exit("launcher activity not found in AndroidManifest.xml")
    end = m.end() - len("</activity>")
    text = text[:end] + filters + text[end:]
    if 'android:launchMode=' not in m.group(0):
        text = text.replace(m.group(0)[: m.group(0).index(">")], m.group(0)[: m.group(0).index(">")] + ' android:launchMode="singleTask"', 1)
    path.write_text(text)


KOTLIN = '''
    private fun receiveShared(intent: Intent?) {
        if (intent == null) return
        val uris = mutableListOf<Uri>()
        @Suppress("DEPRECATION")
        when (intent.action) {
            Intent.ACTION_SEND -> intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)?.let { uris.add(it) }
            Intent.ACTION_SEND_MULTIPLE -> intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)?.let { uris.addAll(it) }
            Intent.ACTION_VIEW -> intent.data?.let { uris.add(it) }
            else -> return
        }
        val dir = File(cacheDir, "shared").apply { mkdirs() }
        for (uri in uris.take(500)) {
            try {
                val name = displayName(uri).replace(Regex("[\\\\\\\\/:*?\\"<>|]"), "_").ifEmpty { "shared" }
                var target = File(dir, name)
                var n = 1
                while (target.exists()) target = File(dir, "${n++}-$name")
                contentResolver.openInputStream(uri)?.use { input -> target.outputStream().use { input.copyTo(it) } }
            } catch (e: Exception) {
                android.util.Log.w("NexonicClear", "Could not receive a shared file", e)
            }
        }
        intent.action = null
    }

    private fun displayName(uri: Uri): String {
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0)?.let { return it }
        }
        return uri.lastPathSegment?.substringAfterLast('/') ?: ""
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        receiveShared(intent)
    }
'''


def activity(path: Path) -> None:
    text = path.read_text()
    if "receiveShared" in text:
        return
    m = re.search(r"class MainActivity\s*:\s*TauriActivity\(\)[ \t]*(\{)?", text)
    if not m:
        sys.exit("MainActivity class not found")
    if not m.group(1):
        text = text[: m.end()] + " {\n}" + text[m.end():]
    imports = "import android.content.Intent\nimport android.net.Uri\nimport android.provider.OpenableColumns\nimport java.io.File\n"
    if "import android.os.Bundle" not in text:
        imports += "import android.os.Bundle\n"
    text = re.sub(r"^(package [^\n]+\n)", r"\1\n" + imports, text, count=1, flags=re.M)
    if "super.onCreate(savedInstanceState)" in text:
        text = text.replace("super.onCreate(savedInstanceState)", "super.onCreate(savedInstanceState)\n    receiveShared(intent)", 1)
        body = KOTLIN
    else:
        body = "\n    override fun onCreate(savedInstanceState: Bundle?) {\n        super.onCreate(savedInstanceState)\n        receiveShared(intent)\n    }\n" + KOTLIN
    i = text.rindex("}")
    text = text[:i] + body + text[i:]
    path.write_text(text)


def main() -> None:
    manifest(APP / "AndroidManifest.xml")
    found = list((APP / "java").rglob("MainActivity.kt"))
    if len(found) != 1:
        sys.exit(f"expected one MainActivity.kt, found {len(found)}")
    activity(found[0])
    print("Android share sheet added")


if __name__ == "__main__":
    main()
