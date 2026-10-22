#!/bin/sh
# Multi-arch / Universal 2 build script for macOS (Apple Silicon + Intel x86_64).
# Usage: ./scripts/build_macos.sh [--bundle]
set -e
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:$PATH"
export MACOSX_DEPLOYMENT_TARGET=12.0

echo "Kompilácia pre Apple Silicon (aarch64-apple-darwin)..."
cargo build --release --target aarch64-apple-darwin

echo "Kompilácia pre Intel (x86_64-apple-darwin)..."
cargo build --release --target x86_64-apple-darwin

mkdir -p target/release
OUTPUT="target/release/CrowBrowser-universal"

echo "Vytváranie Universal 2 fat binary cez lipo..."
lipo -create -output "$OUTPUT" \
  target/aarch64-apple-darwin/release/crow-browser \
  target/x86_64-apple-darwin/release/crow-browser

cp "$OUTPUT" target/CrowBrowser-universal

lipo -info "$OUTPUT"

if [ "$1" = "--bundle" ] || [ "$1" = "--app" ]; then
  ./scripts/bundle.sh "$OUTPUT"
fi
