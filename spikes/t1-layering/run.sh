#!/usr/bin/env bash
#
# Builds the T1 test app and launches it on macOS.
#
#   ./run.sh           Run the self-test, which takes about a minute. Results land in out/.
#   ./run.sh hand      Open the app to try by hand.
#
# The app and its files run from the caches folder, not from here. This repo sits under ~/Documents,
# which macOS protects, and every build is a new ad-hoc identity. So anything the app touched here
# would show a permission prompt after each rebuild, and block until someone answered it.
#
# It launches through `open`, from a small .app, so that macOS treats this app as the one asking for
# any permission. Started directly from a terminal, a prompt would be attributed to the terminal or
# IDE, and allowing it would grant that access to them.

set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
work=$HOME/Library/Caches/reel-spikes/t1-layering
results=$work/results
out=$here/out
app="$work/T1 Layering.app"
video=$work/1080p60.mp4
mode=${1:-self-test}

mkdir -p "$work" "$out"
cargo build --release --manifest-path "$here/Cargo.toml"

if [ ! -f "$video" ]; then
    echo "Generating the test video..."
    ffmpeg -hide_banner -loglevel error -y -f lavfi -i testsrc2=size=1920x1080:rate=60:duration=30 \
        -c:v libx264 -preset veryfast -crf 20 -pix_fmt yuv420p \
        -colorspace bt709 -color_primaries bt709 -color_trc bt709 "$video"
fi

# Always a new, signed bundle. Launches failed with "Codesigning issue" (launchd error 162) when this
# used to overwrite the binary of the existing one.
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$here/target/release/t1-layering" "$app/Contents/MacOS/t1-layering"
cat > "$app/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>dev.reel.spike.t1-layering</string>
    <key>CFBundleName</key><string>T1 Layering</string>
    <key>CFBundleExecutable</key><string>t1-layering</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
codesign --force --sign - "$app"

args=(--env "T1_FFMPEG=$(command -v ffmpeg)" --env "T1_VIDEO=$video" --env "T1_OUT=$results")
[ "$mode" = self-test ] && args+=(--env T1_SELF_TEST=1)
# `open` doesn't pass on the environment, so forward the settings that vary between experiments.
for name in T1_GRAVITY T1_LATENCY T1_ONLY_RESIZE; do
    if [ -n "${!name:-}" ]; then args+=(--env "$name=${!name}"); fi
done

rm -rf "$results"
mkdir -p "$results"
open -W -n "$app" ${args[@]+"${args[@]}"} --stdout "$results/run.log" --stderr "$results/run.log"
cat "$results/run.log"

[ "$mode" = self-test ] || exit 0

cp "$results"/* "$out"/
# `open -W` always exits 0, so the report is the only record of whether the run passed.
if ! grep -q '"passed": true' "$out/report.json" 2>/dev/null; then
    echo "T1 self-test did not pass. See $out/report.json" >&2
    exit 1
fi
echo "T1 self-test passed. Results are in $out"
