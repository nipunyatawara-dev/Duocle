#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
case "${1:-}" in
  aarch64) name=Duocle-macos-arm64.dmg ;;
  x86_64) name=Duocle-macos-intel.dmg ;;
  *) printf 'Usage: %s aarch64|x86_64\n' "$0" >&2; exit 2 ;;
esac
if ! command -v dmgbuild >/dev/null; then
  printf 'Install dmgbuild and put it on PATH before packaging a DMG.\n' >&2
  exit 1
fi

mkdir -p dist
stage="$(mktemp -d "$PWD/dist/.dmg.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
DUOCLE_DIST_DIR="$stage" ./scripts/package-macos.sh "$1"
dmgbuild -s packaging/macos-dmg.py -D "app=$stage/Duocle.app" \
  -D "background=$PWD/packaging/dmg-background.png" \
  "Duocle Installer" "$stage/$name"
hdiutil verify "$stage/$name"
mv "$stage/$name" "dist/$name"
