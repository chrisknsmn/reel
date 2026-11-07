#!/usr/bin/env bash
#
# Runs the launch bar's self-test on this Mac: scripted clicks on the real bar, recording a test pattern
# through the real recorder process, and ending with the editor open on the result. It needs no
# permissions. A window stays on top for about 15 seconds. Results land in target/bar-test/.

set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
work=$HOME/Library/Caches/reel-spikes/bar
results=$work/results
out=$root/target/bar-test
app="$work/Reel Bar Test.app"

if ioreg -n Root -d1 | grep '"CGSSessionScreenIsLocked"=Yes' > /dev/null; then
    echo "The screen is locked. Unlock it and run this again." >&2
    exit 2
fi

mkdir -p "$work" "$out"
cargo build --release -p reel-recorder --manifest-path "$root/Cargo.toml"
cargo build --release -p reel-app --features self-test --manifest-path "$root/Cargo.toml"

rm -rf "$app" "$results" "$work/data"
mkdir -p "$app/Contents/MacOS" "$results" "$work/data"
cp "$root/target/release/reel-app" "$root/target/release/reel-recorder" "$app/Contents/MacOS/"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>dev.reel.bar-test</string>
    <key>CFBundleName</key><string>Reel Bar Test</string>
    <key>CFBundleExecutable</key><string>reel-app</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$app"

open -W -n "$app" --env REEL_SELF_TEST=bar --env REEL_TEST_SOURCE=1 --env "REEL_DATA=$work/data" \
    --env "REEL_OUT=$results" --stdout "$results/run.log" --stderr "$results/run.log"
cat "$results/run.log"
cp "$results"/* "$out"/

if ! grep -q '"passed":true' "$out/report.json" 2>/dev/null; then
    echo "The bar's self-test did not pass. See $out/report.json" >&2
    exit 1
fi
echo "The bar's self-test passed. Results are in $out"
