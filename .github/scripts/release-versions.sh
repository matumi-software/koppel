#!/usr/bin/env bash
# The released version appears by hand in the three composes, the README, docs/windows.md and
# docs/installers.md. The build reads it from src/Cargo.toml only, so a release has to find them all moved.
#
#   .github/scripts/release-versions.sh check [<version>]   fail and list every place that names another version
#   .github/scripts/release-versions.sh write [<version>]   rewrite those places to the version
#
# Without <version> it takes the workspace version from src/Cargo.toml. The places are the image tag
# (`matumisoftware/koppel:<version>`), the .deb file names and release download path in the README, and the
# README's "version <version> is the latest release". Prose such as "a 0.2.0 machine" is not touched: it
# names a version on purpose.
set -eu
cd "$(dirname "$0")/../.."

mode=${1:-}
case "$mode" in check | write) ;; *) echo "usage: $0 check|write [<version>]" >&2; exit 2 ;; esac
version=${2:-$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' src/Cargo.toml | head -1)}
[ -n "$version" ] || { echo "No version found in src/Cargo.toml." >&2; exit 2; }

ver='[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]*[0-9A-Za-z])?'
files="README.md docs/windows.md docs/installers.md src/installer/docker/docker-compose.yml
src/installer/docker/docker-compose.windows-client.yml src/installer/docker/docker-compose.windows-server.yml"
patterns=(
  "matumisoftware/koppel:$ver"
  "koppel_${ver}_(amd64|arm64|armhf)\.deb"
  "releases/download/v$ver/"
  "version $ver is the latest release"
)

if [ "$mode" = write ]; then
  # Each pattern's version is the only part that changes, so rewrite it with the same lead-in and tail.
  for f in $files; do
    sed -E -i \
      -e "s#(matumisoftware/koppel:)$ver#\1$version#g" \
      -e "s#(koppel_)${ver}(_(amd64|arm64|armhf)\.deb)#\1$version\3#g" \
      -e "s#(releases/download/v)$ver/#\1$version/#g" \
      -e "s#(version )$ver( is the latest release)#\1$version\3#g" \
      "$f"
  done
  echo "Wrote $version."
fi

# Compare every match with the version. Anything left over is a place that still names another one.
fail=0
for f in $files; do
  for p in "${patterns[@]}"; do
    while IFS= read -r hit; do
      [ -n "$hit" ] || continue
      found=$(printf '%s' "$hit" | grep -oE "$ver" | head -1)
      if [ "$found" != "$version" ]; then
        echo "::error file=$f::$f names $found ('$hit'), the release is $version."
        fail=1
      fi
    done < <(grep -oE "$p" "$f" || true)
  done
done
[ "$fail" -eq 0 ] && echo "Every place names $version."
exit "$fail"
