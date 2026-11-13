#!/usr/bin/env bash
# The released version appears by hand in the three composes, the README, docs/windows.md and
# docs/installers.md. The builds read it from src/Cargo.toml only, so a release has to find them all moved.
# Run it by hand before tagging (see "To release" in docs/building.md):
#
#   src/installer/release-versions.sh check [<version>]   fail and list what disagrees
#   src/installer/release-versions.sh write [<version>]   rewrite the version strings, then check
#
# Without <version> it takes the workspace version from src/Cargo.toml. The strings are the image tag
# (`matumisoftware/koppel:<version>`), the .deb and .tar.gz file names and release download path in the README, and the
# README's "version <version> is the latest release". Prose such as "a 0.2.0 machine" is not touched: it
# names a version on purpose. `check` also needs a `## <version>` heading in CHANGELOG.md and a first `koppel (<version>)` entry in
# src/installer/linux/changelog (the .deb's changelog), and fails when a
# local tag v<version> exists but is not annotated.
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
  "koppel_${ver}_(amd64|arm64|armhf)\.(deb|tar\.gz)"
  "releases/download/v$ver/"
  "version $ver is the latest release"
)

if [ "$mode" = write ]; then
  for f in $files; do
    sed -E -i \
      -e "s#(matumisoftware/koppel:)$ver#\1$version#g" \
      -e "s#(koppel_)${ver}(_(amd64|arm64|armhf)\.(deb|tar\.gz))#\1$version\3#g" \
      -e "s#(releases/download/v)$ver/#\1$version/#g" \
      -e "s#(version )$ver( is the latest release)#\1$version\3#g" \
      "$f"
  done
  echo "Wrote $version."
fi

fail=0
for f in $files; do
  for p in "${patterns[@]}"; do
    while IFS= read -r hit; do
      [ -n "$hit" ] || continue
      found=$(printf '%s' "$hit" | grep -oE "$ver" | head -1)
      if [ "$found" != "$version" ]; then
        echo "$f names $found ('$hit'), the release is $version." >&2
        fail=1
      fi
    done < <(grep -oE "$p" "$f" || true)
  done
done
grep -qE "^## ${version//./\.}( |\$)" CHANGELOG.md || { echo "CHANGELOG.md has no '## $version' section." >&2; fail=1; }
head -n 1 src/installer/linux/changelog | grep -qE "^koppel \(${version//./\.}\) " || { echo "src/installer/linux/changelog does not start with a 'koppel ($version)' entry." >&2; fail=1; }
if git rev-parse -q --verify "refs/tags/v$version" >/dev/null && [ "$(git cat-file -t "refs/tags/v$version")" != tag ]; then
  echo "The local tag v$version is lightweight. Make it annotated: git tag -a v$version -m ..." >&2
  fail=1
fi
[ "$fail" -eq 0 ] && echo "Every place names $version."
exit "$fail"
