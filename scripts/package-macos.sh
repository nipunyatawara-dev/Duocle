#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=14.6
out_dir="${DUOCLE_DIST_DIR:-$PWD/dist}"
mkdir -p "$out_dir"
stage="$(mktemp -d "$out_dir/.macos.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
app="$stage/Duocle.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/Licenses"
cp ui/fonts/*.txt "$app/Contents/Resources/Licenses/"
cp packaging/Info.plist "$app/Contents/Info.plist"

arch="${1:-universal}"
case "$arch" in
  universal)
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
      cargo build --release --locked --target "$target"
    done
    lipo -create target/{aarch64-apple-darwin,x86_64-apple-darwin}/release/duocle -output "$app/Contents/MacOS/duocle"
    ;;
  aarch64|x86_64)
    target="$arch-apple-darwin"
    cargo build --release --locked --target "$target"
    cp "target/$target/release/duocle" "$app/Contents/MacOS/duocle"
    ;;
  *) printf 'Use universal, aarch64 or x86_64.\n' >&2; exit 2 ;;
esac

iconset="$stage/Duocle.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" assets/duocle.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" assets/duocle.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Duocle.icns"
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign "${DUOCLE_SIGN_IDENTITY:--}" --options runtime "$app"
codesign --verify --strict "$app"
ditto -c -k --norsrc --keepParent "$app" "$stage/Duocle-macos-$arch.zip"
if [[ -d "$out_dir/Duocle.app" ]]; then
  rm -rf "$out_dir/Duocle.app"
fi
mv "$app" "$out_dir/Duocle.app"
mv "$stage/Duocle-macos-$arch.zip" "$out_dir/Duocle-macos-$arch.zip"
