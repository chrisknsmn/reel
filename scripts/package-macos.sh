#!/usr/bin/env bash
#
# Builds Reel.app and a DMG in target/package/.
#
#   scripts/package-macos.sh
#
# With nothing set, the app is signed ad hoc, which runs on this Mac but which Gatekeeper refuses
# elsewhere. To make a release:
#
#   REEL_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"   Signs it, with hardened runtime.
#   REEL_NOTARY_PROFILE=reel                                               Notarises and staples the DMG.
#
# The profile is one made first with `xcrun notarytool store-credentials reel`.
#
# It builds for this Mac's architecture. If the Rust targets for both are installed, it builds a universal
# app: rustup target add aarch64-apple-darwin x86_64-apple-darwin

set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
out=$root/target/package
app=$out/Reel.app
identity=${REEL_SIGNING_IDENTITY:--}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)

rm -rf "$app" "$out/Reel-$version.dmg"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

# Whatever Rust targets are installed. Homebrew's Rust only has the native one.
have_target() {
    command -v rustup > /dev/null && rustup target list --installed | grep -q "^$1\$"
}
binaries=(reel-app reel-recorder)
if have_target aarch64-apple-darwin && have_target x86_64-apple-darwin; then
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
        cargo build --release --target "$target" -p reel-app -p reel-recorder --manifest-path "$root/Cargo.toml"
    done
    for binary in "${binaries[@]}"; do
        lipo -create -output "$app/Contents/MacOS/$binary" \
            "$root/target/aarch64-apple-darwin/release/$binary" \
            "$root/target/x86_64-apple-darwin/release/$binary"
    done
else
    echo "Building for $(uname -m) only, since both Rust targets aren't installed."
    cargo build --release -p reel-app -p reel-recorder --manifest-path "$root/Cargo.toml"
    for binary in "${binaries[@]}"; do
        cp "$root/target/release/$binary" "$app/Contents/MacOS/$binary"
    done
fi

sed "s/__VERSION__/$version/g" "$root/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist" > /dev/null
cp "$root/LICENSE" "$root/THIRD-PARTY.md" "$app/Contents/Resources/"

# The icon, at every size macOS asks for.
iconset=$out/Reel.iconset
rm -rf "$iconset"
mkdir -p "$iconset"
sips -s format png "$root/packaging/macos/icon.svg" --out "$out/icon-1024.png" > /dev/null
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$out/icon-1024.png" --out "$iconset/icon_${size}x${size}.png" > /dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$out/icon-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" > /dev/null
done
iconutil -c icns -o "$app/Contents/Resources/Reel.icns" "$iconset"
rm -rf "$iconset" "$out/icon-1024.png"

# Inside out: the helper first, then the app that contains it.
sign() {
    if [ "$identity" = "-" ]; then
        codesign --force --sign - "$1"
    else
        codesign --force --options runtime --timestamp --sign "$identity" \
            --entitlements "$root/packaging/macos/entitlements.plist" "$1"
    fi
}
sign "$app/Contents/MacOS/reel-recorder"
sign "$app"
codesign --verify --deep --strict "$app"

staging=$out/staging
rm -rf "$staging"
mkdir -p "$staging"
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
dmg=$out/Reel-$version.dmg
hdiutil create -quiet -volname Reel -srcfolder "$staging" -ov -format UDZO "$dmg"
rm -rf "$staging"

if [ "$identity" != "-" ]; then
    codesign --force --timestamp --sign "$identity" "$dmg"
fi
if [ -n "${REEL_NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$dmg" --keychain-profile "$REEL_NOTARY_PROFILE" --wait
    xcrun stapler staple "$dmg"
fi

echo "Made $dmg"
if [ "$identity" = "-" ]; then
    echo "It's signed ad hoc, so it's for this Mac only."
fi
