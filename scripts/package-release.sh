#!/bin/sh
set -eu

# ----- GLOBAL VARIABLES -----

ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
readonly ROOT

# ----- PRINT FUNCTIONS -----

status() { printf '%s\n' "$1" >&2; }
warning() { printf 'warning: %s\n' "$1" >&2; }
error() { printf 'error: %s\n' "$1" >&2; exit 1; }

# ----- MAIN -----

main() {
  [ "$#" -eq 0 ] || error "unexpected argument: $1"

  # extract version from Cargo's package ID
  PKGID=$(cargo pkgid -m "$ROOT/Cargo.toml")
  VERSION=${PKGID##*[#@]}
  [ "$VERSION" != "$PKGID" ] || error "unable to resolve Cargo package version"

  # map uname values to Rust targets & GNU tar commands
  case "$(uname -s):$(uname -m)" in
    Linux:x86_64) TARGET=x86_64-unknown-linux-gnu; TAR=tar ;;
    Linux:aarch64) TARGET=aarch64-unknown-linux-gnu; TAR=tar ;;
    Darwin:arm64) TARGET=aarch64-apple-darwin; TAR=gtar ;;
    *) error "unsupported platform: $(uname -s) $(uname -m)" ;;
  esac
  command -v "$TAR" >/dev/null 2>&1 || error "required GNU tar command is unavailable: $TAR"

  ARCHIVE="cozydot-$VERSION-$TARGET.tar.gz"
  mkdir -p "$ROOT/target"

  cargo build -r --target "$TARGET" --target-dir "$ROOT/target" -m "$ROOT/Cargo.toml" --locked
  # normalize tar & gzip metadata for reproducible archives
  "$TAR" -czf "$ROOT/target/$ARCHIVE" \
    --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner --mode=0755 \
    -C "$ROOT/target/$TARGET/release" cozydot
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$ROOT/target" && sha256sum "$ARCHIVE" >"$ARCHIVE.sha256")
  else
    (cd "$ROOT/target" && shasum -a 256 "$ARCHIVE" >"$ARCHIVE.sha256")
  fi
  # print archive path for release workflows
  printf '%s\n' "$ROOT/target/$ARCHIVE"
}

main "$@"
