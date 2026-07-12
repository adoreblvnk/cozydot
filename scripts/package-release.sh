#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"version":"\([^"]*\)".*/\1/p' | head -n 1)"
target_dir="${repo_root}/target/release"
bundle_dir="${repo_root}/target/cozydot-${version}"
archive="${repo_root}/target/cozydot-${version}.tar.gz"

cargo build --release --locked
rm -rf "$bundle_dir" "$archive"
mkdir -p "$bundle_dir"
install -Dm755 "$target_dir/cozydot" "$bundle_dir/cozydot"
cp -R "$repo_root/configs" "$bundle_dir/configs"
cp -R "$repo_root/dotfiles" "$bundle_dir/dotfiles"
tar -C "$repo_root/target" -czf "$archive" "cozydot-${version}"
printf '%s\n' "$archive"
