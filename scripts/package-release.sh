#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGE_ID="$(cargo pkgid --manifest-path "$ROOT/Cargo.toml")"
VERSION="${PACKAGE_ID##*[#@]}"
[[ -n $VERSION && $VERSION != "$PACKAGE_ID" ]] || { printf 'unable to resolve Cargo package version\n' >&2; exit 1; }
OUTPUT="${1:-$ROOT/target}"
MACHINE="${COZYDOT_ARCH:-$(uname -m)}"
SYSTEM="$(uname -s)"
case "$SYSTEM:$MACHINE" in
  Linux:x86_64 | Linux:amd64) PLATFORM=linux; ARCH=amd64; TARGET=x86_64-unknown-linux-gnu; TAR=tar ;;
  Linux:aarch64 | Linux:arm64) PLATFORM=linux; ARCH=arm64; TARGET=aarch64-unknown-linux-gnu; TAR=tar ;;
  Darwin:arm64) PLATFORM=macos; ARCH=arm64; TARGET=aarch64-apple-darwin; TAR=gtar ;;
  *) printf 'unsupported platform: %s %s\n' "$SYSTEM" "$MACHINE" >&2; exit 1 ;;
esac
command -v "$TAR" >/dev/null 2>&1 || { printf 'required GNU tar command is unavailable: %s\n' "$TAR" >&2; exit 1; }
ASSET="cozydot-$VERSION-$PLATFORM-$ARCH.tar.gz"
mkdir -p "$ROOT/target"
STAGE="$(mktemp -d "$ROOT/target/.package.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT

CARGO_TARGET_DIR="$ROOT/target" cargo build --release --locked --target "$TARGET" --manifest-path "$ROOT/Cargo.toml"
install -m 0755 "$ROOT/target/$TARGET/release/cozydot" "$STAGE/cozydot"
mkdir -p "$OUTPUT"
"$TAR" --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
  -C "$STAGE" -cf - cozydot | gzip -n >"$OUTPUT/$ASSET"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$OUTPUT" && sha256sum "$ASSET" >"$ASSET.sha256")
else
  (cd "$OUTPUT" && shasum -a 256 "$ASSET" >"$ASSET.sha256")
fi
printf '%s\n' "$OUTPUT/$ASSET"
