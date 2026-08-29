#!/bin/sh
set -eu

# ----- GLOBAL VARIABLES -----

readonly PROG="${0##*/}"
VERSION=""
readonly BASE_URL="${COZYDOT_RELEASE_BASE_URL:-https://github.com/adoreblvnk/cozydot/releases}"
readonly BIN_DIR="$HOME/.local/bin"
TEMP="" BIN_TMP=""

# ----- PRINT FUNCTIONS -----

RESET="" STATUS="" WARNING="" ERROR=""

# https://no-color.org
# https://invisible-island.net/ncurses/terminfo.src.html
if [ -t 2 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
  RESET=$(printf '\033[0m')
  STATUS=$(printf '\033[1;92m')  # bold bright green
  WARNING=$(printf '\033[1;33m') # bold yellow
  ERROR=$(printf '\033[1;91m')   # bold bright red
fi

status() { printf '%b%s%b\n' "$STATUS" "$1" "$RESET" >&2; }
warning() { printf '%bwarning:%b %s\n' "$WARNING" "$RESET" "$1" >&2; }
error() { printf '%berror:%b %s\n' "$ERROR" "$RESET" "$1" >&2; exit 1; }

usage() {
  cat <<EOF
Install cozydot

Usage: $PROG [OPTIONS]

Options:
  -v, --version <VERSION>  Install version [default: latest]
  -h, --help               Print help
EOF
}

# ----- HELPERS -----

cleanup() {
  [ -z "$TEMP" ] || rm -rf "$TEMP"
  [ -z "$BIN_TMP" ] || rm -f "$BIN_TMP"
}

# ----- COMMANDS -----

# ----- MAIN -----

main() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -v | --version)
        [ "$#" -gt 1 ] || error "$1 requires a value"
        VERSION="$2"
        shift 2
        ;;
      -h | --help)
        usage
        return
        ;;
      *) error "unexpected argument: $1" ;;
    esac
  done

  # map uname values to Rust targets
  case "$(uname -s):$(uname -m)" in
    Linux:x86_64) TARGET=x86_64-unknown-linux-gnu ;;
    Linux:aarch64) TARGET=aarch64-unknown-linux-gnu ;;
    Darwin:arm64) TARGET=aarch64-apple-darwin ;;
    *) error "unsupported platform: $(uname -s) $(uname -m)" ;;
  esac

  if [ -z "$VERSION" ]; then
    # resolve latest version from release redirect
    if ! URL=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$BASE_URL/latest" 2>/dev/null); then
      error "unable to resolve latest release"
    fi
    # trim longest prefix ending in /v to get version (eg .../releases/tag/v1.0.0 -> 1.0.0)
    VERSION=${URL##*/v}
    [ "$VERSION" != "$URL" ] || error "unable to parse version from release URL: $URL"
  fi

  ASSET="cozydot-v$VERSION-$TARGET.tar.gz"
  URL="$BASE_URL/download/v$VERSION"
  umask 077
  TEMP=$(mktemp -d "${TMPDIR:-/tmp}/cozydot-install.XXXXXX")
  trap cleanup 0 # remove temp dir on exit

  ARCHIVE="$TEMP/$ASSET"
  curl -fsSL "$URL/$ASSET" -o "$ARCHIVE"
  curl -fsSL "$URL/$ASSET.sha256" -o "$ARCHIVE.sha256"
  if command -v sha256sum >/dev/null 2>&1; then
    if ! (cd "$TEMP" && sha256sum -c "$ASSET.sha256") >/dev/null 2>&1; then
      error "checksum verification failed"
    fi
  elif ! (cd "$TEMP" && shasum -a 256 -c "$ASSET.sha256") >/dev/null 2>&1; then
    error "checksum verification failed"
  fi

  tar -xzf "$ARCHIVE" -C "$TEMP" cozydot
  # reject directories & symlinks before installation
  if [ ! -f "$TEMP/cozydot" ] || [ -L "$TEMP/cozydot" ]; then
    error "missing regular binary"
  fi

  mkdir -p "$BIN_DIR"
  # stage binary in destination dir for atomic posix rename
  BIN_TMP=$(mktemp "$BIN_DIR/.cozydot.XXXXXX")
  cp "$TEMP/cozydot" "$BIN_TMP"
  chmod 0755 "$BIN_TMP"
  mv -f "$BIN_TMP" "$BIN_DIR/cozydot"
  BIN_TMP=""
  status "Installed cozydot $VERSION to $BIN_DIR/cozydot"
}

main "$@"
