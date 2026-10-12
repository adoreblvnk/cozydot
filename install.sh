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
  else
    VERSION=${VERSION#v}
  fi
  ASSET="cozydot-$TARGET"
  URL="$BASE_URL/download/v$VERSION"
  umask 077
  TEMP=$(mktemp -d "${TMPDIR:-/tmp}/cozydot-install.XXXXXX")
  trap cleanup 0 # remove temp dir on exit

  BIN="$TEMP/$ASSET"
  if ! curl -fsSL "$URL/$ASSET" -o "$BIN" 2>/dev/null; then
    error "unable to download cozydot release $VERSION from $URL/$ASSET"
  fi
  curl -fsSL "$URL/$ASSET.sha256" -o "$BIN.sha256" 2>/dev/null
  SHACMD="sha256sum"
  command -v sha256sum >/dev/null 2>&1 || SHACMD="shasum -a 256"
  (cd "$TEMP" && $SHACMD -c "$ASSET.sha256") >/dev/null 2>&1 || error "checksum verification failed"

  mkdir -p "$BIN_DIR"
  # stage binary in destination dir for atomic posix rename
  BIN_TMP=$(mktemp "$BIN_DIR/.cozydot.XXXXXX")
  cp "$BIN" "$BIN_TMP"
  chmod 0755 "$BIN_TMP"
  mv -f "$BIN_TMP" "$BIN_DIR/cozydot"
  BIN_TMP=""
  status "Installed cozydot $VERSION to $BIN_DIR/cozydot"
}

main "$@"
