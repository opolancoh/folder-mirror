#!/usr/bin/env sh
# Builds a release on macOS or Linux into dist/<version>/:
#   macOS:  mirror-<version>-macos-arm64 and mirror-<version>-macos-x64
#   Linux:  mirror-<version>-linux-x64 (or -linux-arm64)
#   SHA256SUMS.txt covers every file in the folder (including Windows files copied in).
#
# Run:  sh scripts/release.sh
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)"
out="dist/$version"
mkdir -p "$out"
echo "Release $version -> $root/$out"

build() { # build <rust target> <output name>
  echo
  echo "== $2"
  rustup target add "$1" >/dev/null
  cargo build --release --target "$1"
  cp "target/$1/release/mirror" "$out/$2"
  chmod +x "$out/$2"
}

case "$(uname -s)" in
  Darwin)
    build aarch64-apple-darwin "mirror-$version-macos-arm64"
    build x86_64-apple-darwin "mirror-$version-macos-x64"
    ;;
  Linux)
    case "$(uname -m)" in
      x86_64 | amd64) build x86_64-unknown-linux-gnu "mirror-$version-linux-x64" ;;
      aarch64 | arm64) build aarch64-unknown-linux-gnu "mirror-$version-linux-arm64" ;;
      *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "Use scripts/release.ps1 on Windows." >&2
    exit 1
    ;;
esac

# Checksums for everything in the folder
cd "$out"
if command -v sha256sum >/dev/null 2>&1; then hash="sha256sum"; else hash="shasum -a 256"; fi
find . -maxdepth 1 -type f ! -name SHA256SUMS.txt -exec basename {} \; | sort | while read -r file; do
  $hash "$file"
done > SHA256SUMS.txt

echo
echo "Files in $root/$out"
ls -l
