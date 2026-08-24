#!/bin/sh
set -eu

# ----- GLOBAL VARIABLES -----

readonly PROG="${0##*/}"
RELEASE="${COZYDOT_VERSION:-1.0.0}"
readonly BASE_URL="${COZYDOT_RELEASE_BASE_URL:-https://github.com/adoreblvnk/cozydot/releases}"
readonly BIN_DIR="$HOME/.local/bin"
WORK=""
BIN_TMP=""

# ----- PRINT FUNCTIONS -----

RESET=""
STATUS=""
WARNING=""
ERROR=""

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
Install Cozydot

Usage: $PROG [OPTIONS]

Options:
  -r, --release <VERSION>  Install release version [default: $RELEASE]
  -h, --help               Print help
EOF
}

# ----- HELPERS -----

cleanup() {
  [ -z "$WORK" ] || rm -rf "$WORK"
  [ -z "$BIN_TMP" ] || rm -f "$BIN_TMP"
}

# ----- COMMANDS -----

install_release() {
  KERNEL=$(uname -s)
  MACHINE=$(uname -m)
  case "$KERNEL:$MACHINE" in
    Linux:x86_64) TARGET=x86_64-unknown-linux-gnu ;;
    Linux:aarch64) TARGET=aarch64-unknown-linux-gnu ;;
    Darwin:arm64) TARGET=aarch64-apple-darwin ;;
    *) error "unsupported platform $KERNEL:$MACHINE; supported platforms: Linux x86_64/aarch64, macOS arm64" ;;
  esac

  VERSION=${RELEASE#v}
  ASSET="cozydot-$VERSION-$TARGET.tar.gz"
  URL="$BASE_URL/download/v$VERSION"
  umask 077
  WORK=$(mktemp -d "${TMPDIR:-/tmp}/cozydot-install.XXXXXX")
  trap cleanup 0 # remove temp dir on exit

  ARCHIVE="$WORK/$ASSET"
  curl -fsSL "$URL/$ASSET" -o "$ARCHIVE"
  curl -fsSL "$URL/$ASSET.sha256" -o "$ARCHIVE.sha256"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$WORK" && sha256sum -c "$ASSET.sha256") >/dev/null || error "checksum verification failed"
  else
    (cd "$WORK" && shasum -a 256 -c "$ASSET.sha256") >/dev/null || error "checksum verification failed"
  fi

  tar -tzf "$ARCHIVE" >"$WORK/members"
  [ "$(cat "$WORK/members")" = cozydot ] || error "release must contain exactly one cozydot entry"
  tar -tvzf "$ARCHIVE" >"$WORK/listing"
  [ "$(wc -l <"$WORK/listing")" -eq 1 ] || error "cozydot release entry is not a regular file"
  # GNU tar prefixes regular-file listings with -
  case "$(cat "$WORK/listing")" in
    -*) ;;
    *) error "cozydot release entry is not a regular file" ;;
  esac

  tar --no-same-owner --no-same-permissions -xzf "$ARCHIVE" -C "$WORK" cozydot
  [ -f "$WORK/cozydot" ] && [ ! -L "$WORK/cozydot" ] || error "missing binary"
  mkdir -p "$BIN_DIR"
  BIN_TMP=$(mktemp "$BIN_DIR/.cozydot.XXXXXX")
  cp "$WORK/cozydot" "$BIN_TMP"
  chmod 0755 "$BIN_TMP"
  mv -f "$BIN_TMP" "$BIN_DIR/cozydot"
  BIN_TMP=""
  status "Installed cozydot $VERSION to $BIN_DIR/cozydot"
}

# ----- MAIN -----

main() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -r | --release)
        [ "$#" -gt 1 ] || error "$1 requires a value"
        RELEASE="$2"
        shift 2
        ;;
      -h | --help)
        usage
        return
        ;;
      *) error "unexpected argument: $1" ;;
    esac
  done
  install_release
}

main "$@"
