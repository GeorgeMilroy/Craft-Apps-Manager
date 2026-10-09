#!/bin/bash
# Builds the universal (Apple silicon and Intel) app and packages it as
# target/installer/ArtCraftAppsManager-<version>-macos-universal.dmg.
#
# Runs on a Mac with Xcode or its command line tools, and Rust's two Mac
# targets (one-time):
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
#
# The app is signed ad hoc, not with a Developer ID, so the first time it's
# opened macOS asks for confirmation in System Settings > Privacy & Security.
set -euo pipefail

cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
name="ArtCraft Apps Manager"
exe=artcraft-apps-manager
export MACOSX_DEPLOYMENT_TARGET=11.0

for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --target "$target"
done

out=target/macos
app="$out/$name.app"
rm -rf "$out"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/$exe" \
    "target/aarch64-apple-darwin/release/$exe" \
    "target/x86_64-apple-darwin/release/$exe"
sed "s/{{version}}/$version/g" installer/macos/Info.plist > "$app/Contents/Info.plist"

# The icon, from the 256 px logo. Larger sizes are left out rather than upscaled.
iconset="$out/AppIcon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256; do
    sips -z "$size" "$size" assets/logo.png --out "$iconset/icon_${size}x${size}.png" > /dev/null
done
for size in 16 32 128; do
    double=$((size * 2))
    sips -z "$double" "$double" assets/logo.png --out "$iconset/icon_${size}x${size}@2x.png" > /dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
rm -rf "$iconset"

codesign --force --deep --sign - "$app"
codesign --verify --strict "$app"

# The disk image: the app, and a link to drag it onto.
stage="$out/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
mkdir -p target/installer
dmg="target/installer/ArtCraftAppsManager-$version-macos-universal.dmg"
rm -f "$dmg"
hdiutil create -volname "$name" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$dmg" > /dev/null
rm -rf "$stage"

lipo -info "$app/Contents/MacOS/$exe"
echo "Built $dmg"
