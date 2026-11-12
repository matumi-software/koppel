#!/usr/bin/env bash
# Prints the CHANGELOG.md section of a version, without its heading: .github/scripts/changelog-section.sh 0.2.1
# Fails when CHANGELOG.md has no "## <version>" heading or the section is empty.
set -eu
cd "$(dirname "$0")/../.."
version=${1:?usage: $0 <version>}
body=$(awk -v v="$version" '
  /^## / { if (on) exit; on = ($2 == v) ; next }
  on { print }
' CHANGELOG.md | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}')
if ! grep -qE "^## ${version//./\.}( |$)" CHANGELOG.md; then echo "CHANGELOG.md has no section for $version." >&2; exit 1; fi
[ -n "$(printf '%s' "$body" | tr -d '[:space:]')" ] || { echo "The CHANGELOG.md section for $version is empty." >&2; exit 1; }
printf '%s\n' "$body" | sed '/./,$!d'
