#!/usr/bin/env bash
set -euo pipefail

VERSION="${COZYDOT_VERSION:-0.0.1}"
BASE_URL="${COZYDOT_RELEASE_BASE_URL:-https://github.com/adoreblvnk/cozydot/releases}"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/cozydot"

case "$(uname -s):$(uname -m)" in
  Linux:x86_64) ARCH=amd64 ;;
  Linux:aarch64 | Linux:arm64) ARCH=arm64 ;;
  *) printf 'cozydot: unsupported platform\n' >&2; exit 1 ;;
esac

ASSET="cozydot-${VERSION#v}-linux-$ARCH.tar.gz"
URL="$BASE_URL/download/v${VERSION#v}"
mkdir -p "$BIN_DIR" "$CACHE_DIR"
WORK="$(mktemp -d "$CACHE_DIR/.install.XXXXXX")"
BIN_TMP=""
cleanup() { rm -rf "$WORK"; [[ -z $BIN_TMP ]] || rm -f "$BIN_TMP"; }
trap cleanup EXIT

valid_pair() {
  local archive=$1 checksum=$2 expected actual
  [[ -f $archive && -f $checksum ]] || return 1
  read -r expected _ <"$checksum" || return 1
  [[ $expected =~ ^[[:xdigit:]]{64}$ ]] || return 1
  actual="$(sha256sum "$archive")"
  [[ ${actual%% *} == "$expected" ]]
}

ARCHIVE="$CACHE_DIR/$ASSET"
CHECKSUM="$ARCHIVE.sha256"
PRIVATE_ARCHIVE="$WORK/release.tar.gz"
PRIVATE_CHECKSUM="$WORK/release.tar.gz.sha256"
if valid_pair "$ARCHIVE" "$CHECKSUM"; then
  cp "$ARCHIVE" "$PRIVATE_ARCHIVE"
  cp "$CHECKSUM" "$PRIVATE_CHECKSUM"
  chmod 0600 "$PRIVATE_ARCHIVE" "$PRIVATE_CHECKSUM"
fi
if ! valid_pair "$PRIVATE_ARCHIVE" "$PRIVATE_CHECKSUM"; then
  rm -f "$PRIVATE_ARCHIVE" "$PRIVATE_CHECKSUM"
  rm -f "$ARCHIVE" "$CHECKSUM"
  curl -fsSL "$URL/$ASSET" -o "$PRIVATE_ARCHIVE"
  curl -fsSL "$URL/$ASSET.sha256" -o "$PRIVATE_CHECKSUM"
  chmod 0600 "$PRIVATE_ARCHIVE" "$PRIVATE_CHECKSUM"
  valid_pair "$PRIVATE_ARCHIVE" "$PRIVATE_CHECKSUM" || {
    printf 'cozydot: checksum verification failed\n' >&2
    exit 1
  }
  cp "$PRIVATE_ARCHIVE" "$ARCHIVE"
  if [[ -n ${COZYDOT_TEST_FAIL_CACHE_AFTER_ARCHIVE:-} ]]; then
    printf 'cozydot: injected cache publication failure\n' >&2
    exit 1
  fi
  cp "$PRIVATE_CHECKSUM" "$CHECKSUM"
fi

declare -A MEMBERS=() TYPES=()
BINARY_COUNT=0 CONFIG_COUNT=0
while IFS= read -r member; do
  normalized=${member%/}
  [[ -n $normalized && $normalized != /* && "/$normalized/" != *"/../"* && "/$normalized/" != *"/./"* ]] || {
    printf 'cozydot: unsafe release path: %s\n' "$member" >&2; exit 1;
  }
  case $normalized in cozydot | configs | configs/default.yaml | dotfiles | dotfiles/*) ;; *)
    printf 'cozydot: unexpected release path: %s\n' "$member" >&2; exit 1 ;; esac
  [[ -z ${MEMBERS[$normalized]+x} ]] || { printf 'cozydot: duplicate release path\n' >&2; exit 1; }
  MEMBERS[$normalized]=1
  [[ $member == */ ]] && TYPES[$normalized]=d || TYPES[$normalized]=f
  ancestor=$normalized
  while [[ $ancestor == */* ]]; do
    ancestor=${ancestor%/*}
    [[ ${TYPES[$ancestor]:-d} == d ]] || { printf 'cozydot: release path collision\n' >&2; exit 1; }
  done
  if [[ ${TYPES[$normalized]} == f ]]; then
    for existing in "${!MEMBERS[@]}"; do
      [[ $existing != "$normalized"/* ]] || { printf 'cozydot: release path collision\n' >&2; exit 1; }
    done
  fi
  [[ $normalized == cozydot ]] && ((BINARY_COUNT += 1))
  [[ $normalized == configs/default.yaml ]] && ((CONFIG_COUNT += 1))
done < <(tar -tzf "$PRIVATE_ARCHIVE")
[[ $BINARY_COUNT == 1 && $CONFIG_COUNT == 1 ]] || { printf 'cozydot: incomplete release\n' >&2; exit 1; }
while IFS= read -r listing; do
  [[ ${listing:0:1} == - || ${listing:0:1} == d ]] || { printf 'cozydot: release contains links or special files\n' >&2; exit 1; }
done < <(tar -tvzf "$PRIVATE_ARCHIVE")

tar --no-same-owner --no-same-permissions -xzf "$PRIVATE_ARCHIVE" -C "$WORK" cozydot
[[ -f $WORK/cozydot && ! -L $WORK/cozydot ]] || { printf 'cozydot: missing binary\n' >&2; exit 1; }
BIN_TMP="$(mktemp "$BIN_DIR/.cozydot.XXXXXX")"
cp "$WORK/cozydot" "$BIN_TMP"
chmod 0755 "$BIN_TMP"
mv -f "$BIN_TMP" "$BIN_DIR/cozydot"
BIN_TMP=""
printf 'Installed cozydot %s to %s\n' "${VERSION#v}" "$BIN_DIR/cozydot"
