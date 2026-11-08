#!/usr/bin/env bash
# Runs the app's built-in acceptance path on an emulator: a signed C2PA sample
# is placed in the app's private folder, the real interface inspects, cleans
# and re-verifies it over IPC, and the result is read back.
set -euo pipefail
APK=src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
[ -f "$APK" ] || APK=$(ls src-tauri/gen/android/app/build/outputs/apk/*/debug/*.apk | head -1)
adb install -r "$APK"
adb shell pm clear "$PKG"
adb push crates/clear-core/tests/fixtures/CA.jpg /data/local/tmp/sample.jpg
for d in files/selftest selftest; do
  adb shell "run-as $PKG mkdir -p $d && cat /data/local/tmp/sample.jpg | run-as $PKG sh -c 'cat > $d/sample.jpg'"
done
adb shell am start -W -n "$PKG/.MainActivity"
mkdir -p test-results
for i in $(seq 1 60); do
  for d in files/selftest selftest; do
    if out=$(adb shell "run-as $PKG cat $d/result.json" 2>/dev/null) && [[ "$out" == \{* ]]; then
      echo "Android self-test: $out"
      echo "$out" > test-results/android-selftest.json
      grep -q '"passed":true' test-results/android-selftest.json && exit 0
      exit 1
    fi
  done
  sleep 2
done
adb logcat -d | grep -iE "nexonic|tauri|panic|chromium" | tail -80
echo "No self-test result" && exit 1
