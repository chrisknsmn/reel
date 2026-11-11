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
    # Speech on a mic track, for generating captions, if there's a model to do it with. Getting one:
    #   cargo test -p reel-captions --features whisper -- --ignored
    speech_env=()
    model=$HOME/Library/Caches/reel-spikes/models/ggml-tiny.en.bin
    rm -f "$video.mic.m4a"
    if [ "${REEL_TEST:-}" = editor ] && [ -f "$model" ]; then
        say -o "$work/speech.aiff" "Hello there. This is a test of captions."
        afconvert -f m4af -d aac -c 2 "$work/speech.aiff" "$video.mic.m4a"
        mkdir -p "$work/data"
        printf '{ "whisper_model": "%s" }' "$model" > "$work/data/settings.json"
        speech_env=(--env REEL_TEST_SPEECH=1)
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
    # The project: a gradient, a webcam, an arrow, a blur, some text and a zoom, all 20 s long.
    python3 -I - "$project" "$video" "$webcam" <<'PYTHON'
import json, os, sys
project, video, webcam = sys.argv[1:4]
s = 1_000_000
tracks = []
if os.path.exists(video + ".mic.m4a"):
    tracks.append({"kind": "mic", "file": {"absolute": video + ".mic.m4a"}, "volume": 1.0, "boost": False})
def annotation(id, content, x, y, width, height):
    return {"id": id, "start": 0, "end": 20 * s, "content": content,
            "position": {"x": x, "y": y}, "width": width, "height": height}
json.dump({
    "format": "reel-project", "version": 1, "id": "app-test",
    "media": {"video": {"absolute": video}, "cursor_data": {"absolute": video + ".cursor.json"},
              "source_tracks": tracks},
    "look": {"background": {"kind": "gradient", "value": "linear-gradient(135deg, #1e3a8a, #9333ea)"},
             "shadow": 0.67, "corner_radius": 12.5, "aspect_ratio": "16:9"},
    "cursor": {"click_effect": "echo"},
    "webcam": {"enabled": True, "source": {"absolute": webcam}, "mirror": True,
               "position": "bottom_right", "width": 25, "height": 25},
    "timeline": {
        "source_duration": 20 * s,
        "clips": [{"id": "c", "start": 0, "duration": 20 * s, "source_start": 0, "speed": 1.0}],
        "zooms": [{"id": "z", "start": 3 * s, "end": 7 * s, "depth": 2, "focus": {"x": 0.7, "y": 0.3},
                   "mode": "manual"}],
        "annotations": [
            annotation("a", {"kind": "arrow", "direction": "right", "color": "#FFFFFF", "stroke_width": 6}, 5, 5, 20, 25),
            annotation("b", {"kind": "blur", "strength": 18}, 40, 60, 25, 25),
            annotation("t", {"kind": "text", "text": "Reel"}, 60, 8, 25, 15),
        ],
    },
}, open(project, "w"), indent=2)
PYTHON
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
    ${speech_env[@]+"${speech_env[@]}"} \
    --stdout "$results/run.log" --stderr "$results/run.log" --args "$project"
cat "$results/run.log"
cp "$results"/* "$out"/

if ! grep -Eq '"passed": ?true' "$out/report.json" 2>/dev/null; then
    echo "The app's self-test did not pass. See $out/report.json" >&2
    exit 1
fi
echo "The app's self-test passed. Results are in $out"
