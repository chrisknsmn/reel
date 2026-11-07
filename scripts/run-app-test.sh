#!/usr/bin/env bash
#
# Runs reel-app's self-test on this Mac: scripted clicks on the real UI, with real measurements.
#
#   scripts/run-app-test.sh [project.reel]
#   REEL_TEST=editor scripts/run-app-test.sh    Clicks through the whole editor instead.
#
# A window stays on top for about 20 seconds. Results land in target/app-test/.
#
# The app runs from the caches folder through `open`. The repo is under ~/Documents, which macOS
# protects, so running from here would show a permission prompt after every rebuild. Any project
# given needs its media outside ~/Documents too, for the same reason.

set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
work=$HOME/Library/Caches/reel-spikes/app
results=$work/results
out=$root/target/app-test
app="$work/Reel Test.app"

# Not `grep -q`: it exits at the first match, and pipefail then reports the pipeline as failed.
if ioreg -n Root -d1 | grep '"CGSSessionScreenIsLocked"=Yes' > /dev/null; then
    echo "The screen is locked. Unlock it and run this again." >&2
    exit 2
fi

mkdir -p "$work" "$out"
cargo build --release -p reel-recorder --manifest-path "$root/Cargo.toml"
cargo build --release -p reel-app --features self-test --manifest-path "$root/Cargo.toml"

project=${1:-$work/test.reel}
if [ "$#" -eq 0 ]; then
    video=$work/test.mp4
    if [ ! -f "$video" ]; then
        ffmpeg -hide_banner -loglevel error -y -f lavfi -i testsrc2=size=1920x1080:rate=60:duration=20 \
            -f lavfi -i "sine=frequency=440:duration=20" -c:v libx264 -preset veryfast -crf 20 \
            -pix_fmt yuv420p -colorspace bt709 -color_primaries bt709 -color_trc bt709 -c:a aac -b:a 160k "$video"
    fi
    webcam=$work/test-webcam.mp4
    if [ ! -f "$webcam" ]; then
        ffmpeg -hide_banner -loglevel error -y -f lavfi -i testsrc=size=640x360:rate=30:duration=20 \
            -c:v libx264 -preset veryfast -crf 20 -pix_fmt yuv420p \
            -colorspace bt709 -color_primaries bt709 -color_trc bt709 "$webcam"
    fi
    # A cursor that wanders and clicks, so the comparison with the export path covers it too.
    python3 -I - "$video.cursor.json" <<'PYTHON'
import json, math, sys
samples = []
for index in range(int(20000 / 33)):
    ms = index * 33
    samples.append({
        "timeMs": ms,
        "cx": 0.5 + 0.3 * math.sin(ms / 1700),
        "cy": 0.5 + 0.3 * math.cos(ms / 2300),
        "interactionType": "click" if index % 90 == 45 else "move",
        "cursorType": "arrow",
    })
json.dump({"version": 2, "provider": "native", "samples": samples}, open(sys.argv[1], "w"))
PYTHON
    cat > "$work/test.recordly" <<JSON
{ "version": 1, "videoPath": "$video", "editor": {
  "wallpaper": "linear-gradient(135deg, #1e3a8a, #9333ea)", "shadowIntensity": 0.67, "borderRadius": 12.5,
  "padding": 20, "aspectRatio": "16:9", "cursorClickEffect": "echo",
  "webcam": { "enabled": true, "sourcePath": "$webcam", "mirror": true, "positionPreset": "bottom-right",
    "size": 25, "width": 25, "height": 25 },
  "annotationRegions": [
    { "id": "a", "startMs": 0, "endMs": 20000, "type": "figure", "x": 5, "y": 5, "width": 20, "height": 25,
      "position": { "x": 5, "y": 5 }, "size": { "width": 20, "height": 25 },
      "figureData": { "arrowDirection": "right", "color": "#FFFFFF", "strokeWidth": 6 } },
    { "id": "b", "startMs": 0, "endMs": 20000, "type": "blur", "blurIntensity": 18,
      "position": { "x": 40, "y": 60 }, "size": { "width": 25, "height": 25 } },
    { "id": "t", "startMs": 0, "endMs": 20000, "type": "text", "textContent": "Reel",
      "position": { "x": 60, "y": 8 }, "size": { "width": 25, "height": 15 } } ],
  "zoomRegions": [{ "id": "z", "startMs": 3000, "endMs": 7000, "depth": 3, "focus": { "cx": 0.7, "cy": 0.3 }, "mode": "manual" }],
  "clipRegions": [{ "id": "c", "startMs": 0, "endMs": 20000, "speed": 1 }] } }
JSON
    "$root/target/release/reel-cli" convert "$work/test.recordly" "$project" --force > /dev/null
fi

rm -rf "$app" "$results"
mkdir -p "$app/Contents/MacOS" "$results"
cp "$root/target/release/reel-app" "$root/target/release/reel-recorder" "$app/Contents/MacOS/"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>dev.reel.app-test</string>
    <key>CFBundleName</key><string>Reel Test</string>
    <key>CFBundleExecutable</key><string>reel-app</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$app"

open -W -n "$app" --env "REEL_SELF_TEST=${REEL_TEST:-1}" --env "REEL_DATA=$work/data" --env "REEL_OUT=$results" \
    --stdout "$results/run.log" --stderr "$results/run.log" --args "$project"
cat "$results/run.log"
cp "$results"/* "$out"/

if ! grep -Eq '"passed": ?true' "$out/report.json" 2>/dev/null; then
    echo "The app's self-test did not pass. See $out/report.json" >&2
    exit 1
fi
echo "The app's self-test passed. Results are in $out"
