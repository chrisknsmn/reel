#!/usr/bin/env bash
#
# Records the Recordly export baseline for BUILD_PLAN step 0.5, using its headless export (§55).
#
# Everything goes in baseline/, which git ignores: sources, projects, exports and timings.
# Recordly gets its own data folder there, so your settings and recordings aren't touched.
# Its editor window appears during each export, and it may ask for camera and mic access at launch.
#
# Needs ffmpeg and a built Recordly. Set RECORDLY_APP to use a different build.

set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
app=${RECORDLY_APP:-$HOME/Projects/Recordly/release/mac-arm64/Recordly.app}
out=$root/baseline
duration=30
timeout_seconds=1200

[ -d "$app" ] || { echo "Recordly not found at $app. Build it, or set RECORDLY_APP." >&2; exit 1; }
# A fresh data folder each time, so caches from an earlier run can't affect the timings.
rm -rf "$out/recordly-data"
mkdir -p "$out/sources" "$out/projects" "$out/exports" "$out/logs"
: > "$out/results.tsv"
printf 'project\tsuccess\tphase\telapsed_ms\trealtime_factor\tbackend\tencoding\n' >> "$out/results.tsv"

make_video() { # path, lavfi source, include sine audio (1 or 0)
    [ -f "$1" ] && return 0
    local audio=()
    [ "$3" = 1 ] && audio=(-f lavfi -i "sine=frequency=440:duration=$duration" -c:a aac -b:a 160k)
    # The odd expansion is for macOS's bash 3.2, where an empty array counts as unbound under set -u.
    ffmpeg -hide_banner -loglevel error -y -f lavfi -i "$2" ${audio[@]+"${audio[@]}"} \
        -c:v libx264 -preset veryfast -crf 20 -pix_fmt yuv420p \
        -colorspace bt709 -color_primaries bt709 -color_trc bt709 -movflags +faststart "$1"
}

write_project() { # path, video path, webcam JSON
    cat > "$1" <<EOF
{
  "version": 1,
  "videoPath": "$2",
  "editor": {
    "wallpaper": "#20303f",
    "shadowIntensity": 0.6,
    "backgroundBlur": 0,
    "borderRadius": 12,
    "padding": { "top": 10, "bottom": 10, "left": 10, "right": 10, "linked": true },
    "aspectRatio": "16:9",
    "zoomRegions": [
      { "id": "zoom-1", "startMs": 5000, "endMs": 11000, "depth": 2, "focus": { "cx": 0.6, "cy": 0.4 } },
      { "id": "zoom-2", "startMs": 18000, "endMs": 25000, "depth": 3, "focus": { "cx": 0.3, "cy": 0.7 } }
    ],
    "webcam": $3
  }
}
EOF
}

run_project() { # name, width, height, fps, webcam JSON
    local name=$1 w=$2 h=$3 fps=$4 webcam=$5
    local video=$out/sources/$name.mp4 project=$out/projects/$name.recordly export=$out/exports/$name.mp4

    make_video "$video" "testsrc2=size=${w}x${h}:rate=$fps:duration=$duration" 1
    write_project "$project" "$video" "$webcam"
    # Each run needs its own log: `open` can fail with error -10810 on a leftover one.
    rm -f "$export" "$export.report.json" "$out/logs/$name.log"

    echo "Exporting $name..."
    # Launch through `open` so macOS treats Recordly as the app asking for camera and mic access.
    # Started directly from a terminal, the request counts as the terminal's or IDE's own, so
    # clicking Allow would grant that access to the terminal or IDE instead.
    perl -e 'alarm shift; exec @ARGV' "$timeout_seconds" open -W -n -a "$app" \
        --env RECORDLY_SMOKE_EXPORT=1 \
        --env "RECORDLY_SMOKE_EXPORT_PROJECT=$project" \
        --env "RECORDLY_SMOKE_EXPORT_OUTPUT=$export" \
        --env RECORDLY_SMOKE_EXPORT_QUALITY=source \
        --env "RECORDLY_SMOKE_EXPORT_FPS=$fps" \
        --stdout "$out/logs/$name.log" --stderr "$out/logs/$name.log" \
        --args "--user-data-dir=$out/recordly-data" --use-mock-keychain ||
        { echo "  No clean exit for $name (see logs/$name.log)"; pkill -f "user-data-dir=$out/recordly-data" || true; }

    python3 -I - "$name" "$export.report.json" "$duration" >> "$out/results.tsv" <<'PY'
import json, sys
name, path, duration = sys.argv[1:4]
try:
    r = json.load(open(path))
except OSError:
    print(f"{name}\tfalse\tno report\t\t\t\t")
    raise SystemExit
ms = r.get("elapsedMs")
factor = f"{int(duration) * 1000 / ms:.2f}" if ms else ""
print("\t".join(str(x) for x in [name, r.get("success"), r.get("phase", ""), ms or "", factor,
                                 r.get("backendPreference", ""), r.get("encodingMode", "")]))
PY
}

make_video "$out/sources/webcam.mp4" "testsrc=size=1280x720:rate=30:duration=$duration" 0

run_project 1080p30 1920 1080 30 '{ "enabled": false }'
run_project 1440p60 2560 1440 60 '{ "enabled": false }'
run_project 4k-webcam 3840 2160 30 \
    "{ \"enabled\": true, \"sourcePath\": \"$out/sources/webcam.mp4\", \"size\": 25 }"

echo
cat "$out/results.tsv"
