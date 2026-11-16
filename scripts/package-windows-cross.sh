#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
target=x86_64-pc-windows-gnu
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
export CARGO_TARGET_DIR="$PWD/target/cross-windows"
build="$CARGO_TARGET_DIR/$target/release"

cargo build --release --locked --target "$target"
loader=("$build"/build/webview2-com-sys-*/out/x64/WebView2Loader.dll)
if [[ ! -f "${loader[0]}" ]]; then
  printf 'The WebView2 SDK loader was not found in the Cargo build output.\n' >&2
  exit 1
fi

mkdir -p dist
makensis "-DDUOCLE_EXE=$build/duocle.exe" "-DWEBVIEW2_LOADER=${loader[0]}" \
  "-DOUTPUT_EXE=$PWD/dist/Duocle-windows-x64-setup.exe" packaging/windows.nsi
