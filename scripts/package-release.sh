#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${1:-$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)}"
OUTPUT="${2:-$ROOT/target}"
ARCH="${COZYDOT_ARCH:-$(case "$(uname -m)" in x86_64) printf amd64 ;; aarch64 | arm64) printf arm64 ;; *) exit 1 ;; esac)}"
ASSET="cozydot-$VERSION-linux-$ARCH.tar.gz"
STAGE="$(mktemp -d "$ROOT/target/.package.XXXXXX")"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT

cargo build --release --locked --manifest-path "$ROOT/Cargo.toml"
install -Dm755 "$ROOT/target/release/cozydot" "$STAGE/cozydot"
find "$STAGE" -exec touch -h -d '@0' {} +
mkdir -p "$OUTPUT"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
  -C "$STAGE" -cf - cozydot | gzip -n >"$OUTPUT/$ASSET"
(cd "$OUTPUT" && sha256sum "$ASSET" >"$ASSET.sha256")
printf '%s\n' "$OUTPUT/$ASSET"
