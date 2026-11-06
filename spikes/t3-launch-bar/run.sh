#!/usr/bin/env bash
#
# Builds the T3 test app and launches it on macOS.
#
#   ./run.sh           Run the self-test. Results land in out/.
#   ./run.sh hand      Open the app to try by hand.
#
# Like T1's run.sh, it runs from the caches folder and launches through `open`, so macOS doesn't
# show a permission prompt for this repo's folder after every rebuild.

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=$HOME/Library/Caches/reel-spikes/t3-launch-bar
results=$work/results
out=$here/out
app="$work/T3 Launch Bar.app"
mode=${1:-self-test}

mkdir -p "$work" "$out"
cargo build --release --manifest-path "$here/Cargo.toml"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$here/target/release/t3-launch-bar" "$app/Contents/MacOS/t3-launch-bar"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>dev.reel.spike.t3-launch-bar</string>
    <key>CFBundleName</key><string>T3 Launch Bar</string>
    <key>CFBundleExecutable</key><string>t3-launch-bar</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$app"

args=(--env "T3_OUT=$results")
[ "$mode" = self-test ] && args+=(--env T3_SELF_TEST=1)

rm -rf "$results"
mkdir -p "$results"
open -W -n "$app" "${args[@]}" --stdout "$results/run.log" --stderr "$results/run.log"
cat "$results/run.log"

[ "$mode" = self-test ] || exit 0

cp "$results"/* "$out"/
if ! grep -q '"passed": true' "$out/report.json" 2>/dev/null; then
    echo "T3 self-test did not pass. See $out/report.json" >&2
    exit 1
fi
echo "T3 self-test passed. Results are in $out"
