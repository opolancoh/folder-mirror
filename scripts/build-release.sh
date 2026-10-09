#!/usr/bin/env sh
# Builds the release binary for this Linux or macOS machine and copies it to dist/
# as a single file: dist/mirror-<version>-<os>-<arch>
#
# Run from anywhere:  sh scripts/build-release.sh
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)"
case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  *) os="$(uname -s | tr '[:upper:]' '[:lower:]')" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) arch="$(uname -m)" ;;
esac

cargo build --release

mkdir -p dist
output="dist/mirror-$version-$os-$arch"
cp target/release/mirror "$output"
chmod +x "$output"

echo
echo "Built: $root/$output"
