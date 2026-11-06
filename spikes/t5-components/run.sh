#!/usr/bin/env bash
#
# Builds the T5 test app and launches it on macOS.
#
#   ./run.sh           Run the self-test. Results land in out/.
#   ./run.sh hand      Open the app to try by hand.
#
# Like T1's run.sh, it runs from the caches folder and launches through `open`, so macOS doesn't
# show a permission prompt for this repo's folder after every rebuild.

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=$HOME/Library/Caches/reel-spikes/t5-components
results=$work/results
out=$here/out
app="$work/T5 Components.app"
mode=${1:-self-test}

# Windows can't take focus or be captured while the screen is locked, so results would be meaningless.
# Not `grep -q`: it exits at the first match, and pipefail then reports the pipeline as failed.
if ioreg -n Root -d1 | grep '"CGSSessionScreenIsLocked"=Yes' > /dev/null; then
    echo "The screen is locked. Unlock it and run this again." >&2
    exit 2
fi

mkdir -p "$work" "$out"
cargo build --release --manifest-path "$here/Cargo.toml"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$here/target/release/t5-components" "$app/Contents/MacOS/t5-components"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>dev.reel.spike.t5-components</string>
    <key>CFBundleName</key><string>T5 Components</string>
    <key>CFBundleExecutable</key><string>t5-components</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$app"

args=(--env "T5_OUT=$results")
[ "$mode" = self-test ] && args+=(--env T5_SELF_TEST=1)

rm -rf "$results"
mkdir -p "$results"
open -W -n "$app" "${args[@]}" --stdout "$results/run.log" --stderr "$results/run.log"
cat "$results/run.log"

[ "$mode" = self-test ] || exit 0

cp "$results"/* "$out"/
echo "Results are in $out"
