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

  # map uname values to Rust targets
  case "$(uname -s):$(uname -m)" in
    Linux:x86_64) TARGET=x86_64-unknown-linux-gnu ;;
    Linux:aarch64) TARGET=aarch64-unknown-linux-gnu ;;
    Darwin:arm64) TARGET=aarch64-apple-darwin ;;
    *) error "unsupported platform: $(uname -s) $(uname -m)" ;;
  esac

  ASSET="cozydot-v$VERSION-$TARGET"
  mkdir -p "$ROOT/target"

  cargo build -r --target "$TARGET" --target-dir "$ROOT/target" -m "$ROOT/Cargo.toml" --locked
  cp -f "$ROOT/target/$TARGET/release/cozydot" "$ROOT/target/$ASSET"
  chmod 0755 "$ROOT/target/$ASSET"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$ROOT/target" && sha256sum "$ASSET" >"$ASSET.sha256")
  else
    (cd "$ROOT/target" && shasum -a 256 "$ASSET" >"$ASSET.sha256")
  fi
  # print asset path for release workflows
  printf '%s\n' "$ROOT/target/$ASSET"
}

main "$@"
