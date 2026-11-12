#!/usr/bin/env bash
# Fails when the places that name one tool version or one licence list disagree. Run from anywhere:
#   .github/scripts/check-pins.sh
# CI runs it as the Pins job. It reads files only, so it needs nothing installed.
set -eu
cd "$(dirname "$0")/../.."

fail=0
note() { echo "::error::$*"; fail=1; }
same() { # same <what> <wanted> <file> <found>
  if [ "$2" != "$4" ]; then note "$1: $3 says '$4', but the version's source file says '$2'."; fi
}

dockerfile=src/installer/docker/Dockerfile

# Rust: rust-toolchain.toml is the source; the Dockerfile base image, the docs and rust-version follow.
rust=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' src/rust-toolchain.toml)
[ -n "$rust" ] || note "src/rust-toolchain.toml has no channel."
same "Rust" "$rust" "$dockerfile" "$(sed -n 's/^FROM .* rust:\([0-9.]*\)-trixie.*/\1/p' $dockerfile | head -1)"
msrv=$(sed -n 's/^rust-version *= *"\(.*\)"/\1/p' src/Cargo.toml)
case "$rust" in "$msrv"|"$msrv".*) ;; *) note "Rust: src/Cargo.toml rust-version '$msrv' is not the toolchain's '$rust' major.minor." ;; esac
for f in CLAUDE.md README.md docs/*.md; do
  for v in $(grep -o 'rust:[0-9][0-9.]*-trixie' "$f" | sed 's/^rust://; s/-trixie$//' | sort -u); do
    same "Rust" "$rust" "$f" "$v"
  done
done

# Node: src/ui/.nvmrc is the source.
node=$(tr -d '[:space:]' < src/ui/.nvmrc)
same "Node" "$node" "$dockerfile" "$(sed -n 's/^FROM .* node:\([0-9.]*\)-bookworm-slim.*/\1/p' $dockerfile | head -1)"

# pnpm: the packageManager field is the source.
pnpm=$(sed -n 's/.*"packageManager": *"pnpm@\([0-9.]*\)".*/\1/p' src/ui/package.json)
same "pnpm" "$pnpm" "$dockerfile" "$(sed -n 's/^ARG PNPM_VERSION=\(.*\)/\1/p' $dockerfile)"

# Licences: deny.toml's allow list and about.toml's accepted list name the same licences.
list() { # list <file> <key>: the quoted strings of the first array under that key
  awk -v key="$2" '
    index($0, key " = [") == 1 { on = 1; next }
    on && /^\]/ { exit }
    on { line = $0; sub(/#.*/, "", line); if (match(line, /"[^"]+"/)) print substr(line, RSTART + 1, RLENGTH - 2) }
  ' "$1" | sort
}
deny=$(list src/deny.toml allow)
about=$(list src/installer/notices/about.toml accepted)
[ -n "$deny" ] && [ -n "$about" ] || note "Could not read the licence lists from deny.toml and about.toml."
if [ "$deny" != "$about" ]; then
  note "Licences: deny.toml allow and about.toml accepted differ:"
  diff <(echo "$deny") <(echo "$about") || true
fi

[ "$fail" -eq 0 ] && echo "Pins agree: Rust $rust, Node $node, pnpm $pnpm, $(echo "$deny" | wc -l) licences."
exit "$fail"
