#!/usr/bin/env bash
set -euo pipefail

VERSION="${COZYDOT_VERSION:-0.0.1}"
BASE_URL="${COZYDOT_RELEASE_BASE_URL:-https://github.com/adoreblvnk/cozydot/releases}"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"

case "$(uname -s):$(uname -m)" in
  Linux:x86_64) PLATFORM=linux; ARCH=amd64 ;;
  Linux:aarch64 | Linux:arm64) PLATFORM=linux; ARCH=arm64 ;;
  Linux:armv7 | Linux:armv7l | Linux:armhf) PLATFORM=linux; ARCH=arm32 ;;
  Darwin:x86_64) PLATFORM=macos; ARCH=amd64 ;;
  Darwin:arm64) PLATFORM=macos; ARCH=arm64 ;;
  *) printf 'cozydot: unsupported platform\n' >&2; exit 1 ;;
esac

ASSET="cozydot-${VERSION#v}-$PLATFORM-$ARCH.tar.gz"
URL="$BASE_URL/download/v${VERSION#v}"
umask 077
WORK="$(mktemp -d "${TMPDIR:-/tmp}/cozydot-install.XXXXXX")"
BIN_TMP=""
cleanup() { rm -rf "$WORK"; [[ -z $BIN_TMP ]] || rm -f "$BIN_TMP"; }
trap cleanup EXIT

valid_pair() {
  local archive=$1 checksum=$2 expected actual
  [[ -f $archive && -f $checksum ]] || return 1
  read -r expected _ <"$checksum" || return 1
  [[ $expected =~ ^[[:xdigit:]]{64}$ ]] || return 1
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$archive")"
  else
    actual="$(shasum -a 256 "$archive")"
  fi
  [[ ${actual%% *} == "$expected" ]]
}

ARCHIVE="$WORK/release.tar.gz"
CHECKSUM="$WORK/release.tar.gz.sha256"
curl -fsSL "$URL/$ASSET" -o "$ARCHIVE"
curl -fsSL "$URL/$ASSET.sha256" -o "$CHECKSUM"
valid_pair "$ARCHIVE" "$CHECKSUM" || {
  printf 'cozydot: checksum verification failed\n' >&2
  exit 1
}

tar -tzf "$ARCHIVE" >"$WORK/members"
mapfile -t MEMBERS <"$WORK/members"
[[ ${#MEMBERS[@]} == 1 && ${MEMBERS[0]} == cozydot ]] || {
  printf 'cozydot: release must contain exactly one cozydot entry\n' >&2
  exit 1
}
tar -tvzf "$ARCHIVE" >"$WORK/listing"
mapfile -t LISTING <"$WORK/listing"
[[ ${#LISTING[@]} == 1 && ${LISTING[0]:0:1} == - ]] || {
  printf 'cozydot: cozydot release entry is not a regular file\n' >&2
  exit 1
}

tar --no-same-owner --no-same-permissions -xzf "$ARCHIVE" -C "$WORK" cozydot
[[ -f $WORK/cozydot && ! -L $WORK/cozydot ]] || { printf 'cozydot: missing binary\n' >&2; exit 1; }
mkdir -p "$BIN_DIR"
BIN_TMP="$(mktemp "$BIN_DIR/.cozydot.XXXXXX")"
cp "$WORK/cozydot" "$BIN_TMP"
chmod 0755 "$BIN_TMP"
mv -f "$BIN_TMP" "$BIN_DIR/cozydot"
BIN_TMP=""
printf 'Installed cozydot %s to %s\n' "${VERSION#v}" "$BIN_DIR/cozydot"
