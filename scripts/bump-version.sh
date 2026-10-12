#!/bin/sh
set -eu

# ----- GLOBAL VARIABLES -----

ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
readonly ROOT

# ----- PRINT FUNCTIONS -----

status() { printf '%s\n' "$1" >&2; }
warning() { printf 'warning: %s\n' "$1" >&2; }
error() { printf 'error: %s\n' "$1" >&2; exit 1; }

usage() {
  cat <<EOF
Bump cozydot version in Cargo.toml & Cargo.lock

Usage: bump-version.sh [patch|minor|major]

Arguments:
  LEVEL  Bump level: patch (default), minor, or major
EOF
}

# ----- MAIN -----

main() {
  LEVEL="patch"
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -h | --help) usage; return ;;
      patch | minor | major) LEVEL="$1"; shift ;;
      *) error "unexpected argument: $1; expected patch, minor, or major" ;;
    esac
  done

  CURRENT=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml")
  [ -n "$CURRENT" ] || error "unable to read version from Cargo.toml"

  MAJOR=$(printf '%s' "$CURRENT" | cut -d. -f1)
  MINOR=$(printf '%s' "$CURRENT" | cut -d. -f2)
  PATCH=$(printf '%s' "$CURRENT" | cut -d. -f3)

  case "$LEVEL" in
    patch) NEW="${MAJOR}.${MINOR}.$((PATCH + 1))" ;;
    minor) NEW="${MAJOR}.$((MINOR + 1)).0" ;;
    major) NEW="$((MAJOR + 1)).0.0" ;;
  esac

  sed -i.bak "s/^version = \"$CURRENT\"/version = \"$NEW\"/" "$ROOT/Cargo.toml"
  rm -f "$ROOT/Cargo.toml.bak"

  cargo check --quiet --manifest-path "$ROOT/Cargo.toml"
  git add "$ROOT/Cargo.toml" "$ROOT/Cargo.lock"
  git commit -m "chore(release): bump version to $NEW"
  status "Bumped & committed cozydot $CURRENT -> $NEW"
}

main "$@"
