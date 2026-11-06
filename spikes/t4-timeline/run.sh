#!/usr/bin/env bash
#
# Builds the T4 test app and runs its self-test on macOS, with DOM items and with a canvas, at 500 and 2,000 items.
# Takes about two minutes, with a window on top. Reports land in out/.
#
#   ./run.sh           Run both self-tests.
#   ./run.sh hand dom  Open a mode (dom or canvas) to try by hand.
#
# Like T1's run.sh, it runs from the caches folder and launches through `open`.

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=$HOME/Library/Caches/reel-spikes/t4-timeline
results=$work/results
out=$here/out
app="$work/T4 Timeline.app"
mode=${1:-self-test}

mkdir -p "$work" "$out"
cargo build --release --manifest-path "$here/Cargo.toml"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$here/target/release/t4-timeline" "$app/Contents/MacOS/t4-timeline"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>dev.reel.spike.t4-timeline</string>
    <key>CFBundleName</key><string>T4 Timeline</string>
    <key>CFBundleExecutable</key><string>t4-timeline</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$app"

launch() { # mode, extra args...
    local which=$1
    shift
    rm -rf "$results"
    mkdir -p "$results"
    open -W -n "$app" --env "T4_MODE=$which" --env "T4_OUT=$results" "$@" \
        --stdout "$results/run.log" --stderr "$results/run.log"
    cat "$results/run.log"
}

if [ "$mode" = hand ]; then
    launch "${2:-dom}"
    exit 0
fi

for setup in dom:500 canvas:500 dom:2000 canvas:2000; do
    which=${setup%%:*}
    items=${setup##*:}
    launch "$which" --env T4_SELF_TEST=1 --env "T4_ITEMS=$items" > /dev/null
    cp "$results"/report-* "$out"/
done
