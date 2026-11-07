#!/usr/bin/env bash
#
# Records for real: your screen, system audio, mic and webcam, for about 16 seconds.
#
#   scripts/run-record-test.sh          Uses whatever's already allowed, and shows no prompts.
#   scripts/run-record-test.sh --ask    Shows macOS's permission prompts for anything not yet answered.
#
# It runs from a small app bundle in the caches folder, through `open`, so that macOS treats this app as
# the one asking. Started from a terminal, a prompt would name the terminal or IDE, and allowing it
# would give them the access.
#
# Screen Recording only takes effect after a restart: allow it in System Settings, then run this again.
# A rebuild counts as a new app to macOS, so this keeps the app when the binary hasn't changed.
#
# The recording is left in the results folder for you to look at, and nothing is sent anywhere.

set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
work=$HOME/Library/Caches/reel-spikes/record-test
results=$work/results
app="$work/Reel Recorder Test.app"
binary=$app/Contents/MacOS/reel-recorder

if ioreg -n Root -d1 | grep '"CGSSessionScreenIsLocked"=Yes' > /dev/null; then
    echo "The screen is locked. Unlock it and run this again." >&2
    exit 2
fi

mkdir -p "$work"
cargo build --release -p reel-recorder --manifest-path "$root/Cargo.toml"

if ! cmp -s "$root/target/release/reel-recorder" "$binary"; then
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS"
    cp "$root/target/release/reel-recorder" "$binary"
    cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>dev.reel.recorder-test</string>
    <key>CFBundleName</key><string>Reel Recorder Test</string>
    <key>CFBundleExecutable</key><string>reel-recorder</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>LSUIElement</key><true/>
    <key>NSMicrophoneUsageDescription</key><string>Reel's recording test records a few seconds from your microphone.</string>
    <key>NSCameraUsageDescription</key><string>Reel's recording test records a few seconds from your camera.</string>
</dict>
</plist>
PLIST
    codesign --force --sign - "$app"
fi

rm -rf "$results"
mkdir -p "$results"
args=(self-test "$results")
if [ "${1:-}" = "--ask" ]; then
    args+=(--ask)
fi
open -W -n "$app" --stdout "$results/run.log" --stderr "$results/run.log" --args "${args[@]}"
cat "$results/run.log"
echo
echo "The recording and thumbnails are in: $results"
if ! grep -q '"passed":true' "$results/report.json" 2>/dev/null; then
    echo "The recording test did not pass." >&2
    exit 1
fi
echo "The recording test passed."
