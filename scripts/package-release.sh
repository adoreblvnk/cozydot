#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${1:-$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)}"
OUTPUT="${2:-$ROOT/target}"
MACHINE="${COZYDOT_ARCH:-$(uname -m)}"
case "$MACHINE" in
  x86_64 | amd64) ARCH=amd64; TARGET=x86_64-unknown-linux-gnu ;;
  aarch64 | arm64) ARCH=arm64; TARGET=aarch64-unknown-linux-gnu ;;
  armv7 | armv7l | armhf | arm32) ARCH=arm32; TARGET=armv7-unknown-linux-gnueabihf ;;
  *) printf 'unsupported architecture: %s\n' "$MACHINE" >&2; exit 1 ;;
esac
ASSET="cozydot-$VERSION-linux-$ARCH.tar.gz"
mkdir -p "$ROOT/target"
STAGE="$(mktemp -d "$ROOT/target/.package.XXXXXX")"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT

CARGO_TARGET_DIR="$ROOT/target" cargo build --release --locked --target "$TARGET" --manifest-path "$ROOT/Cargo.toml"
install -Dm755 "$ROOT/target/$TARGET/release/cozydot" "$STAGE/cozydot"
find "$STAGE" -exec touch -h -d '@0' {} +
mkdir -p "$OUTPUT"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
  -C "$STAGE" -cf - cozydot | gzip -n >"$OUTPUT/$ASSET"
(cd "$OUTPUT" && sha256sum "$ASSET" >"$ASSET.sha256")
printf '%s\n' "$OUTPUT/$ASSET"
