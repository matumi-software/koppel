#!/bin/bash
# Checks the Skakel crates Koppel keeps in src/vendor/skakel: each .crate file against SHA256SUMS (the file of
# Skakel's GitHub release), and each unpacked folder against what its .crate file unpacks to, byte for byte.
# Run it before a pull request that touches src/vendor/skakel or the [patch.crates-io] in src/Cargo.toml.
#   bash .github/scripts/check-vendored-skakel.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
dir="$root/src/vendor/skakel"
fail=0

(cd "$dir" && sha256sum -c --ignore-missing SHA256SUMS) || fail=1

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
for crate in "$dir"/*.crate; do
  name="$(basename "$crate" .crate)"
  mkdir "$scratch/$name"
  tar -xzf "$crate" -C "$scratch"
  if [ ! -d "$dir/$name" ]; then
    echo "MISSING  $name: no unpacked folder in vendor/skakel" >&2
    fail=1
  elif ! diff -r "$scratch/$name" "$dir/$name" > /dev/null; then
    echo "DIFFERS  $name: the folder is not what $name.crate unpacks to" >&2
    fail=1
  else
    echo "$name: folder matches the crate"
  fi
done

# Every folder is named in the patch, and every patch names a folder here.
for folder in "$dir"/*/; do
  name="$(basename "$folder")"
  crate="${name%-*}"
  grep -q "^$crate = { path = \"vendor/skakel/$name\" }" "$root/src/Cargo.toml" || { echo "NOT PATCHED  $name is not in [patch.crates-io]" >&2; fail=1; }
done

[ "$fail" -eq 0 ] && echo "Skakel's vendored crates agree with the release's checksums."
exit "$fail"
