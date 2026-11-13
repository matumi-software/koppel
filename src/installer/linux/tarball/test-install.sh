#!/bin/sh
# Runs install.sh and uninstall.sh from an unpacked tarball in a container, with stand-ins for systemctl and
# modprobe, and checks what a first install, an upgrade, an upgrade of a disabled service, a removal, a purge and a
# machine without systemd do. No systemd runs; the stand-in keeps the unit's enabled link and an "active" flag,
# and logs each call. From the repo, with the tarball unpacked to <dir>:
#
#   docker run --rm -v "<dir>:/pkg:ro" -v "$PWD/src/installer/linux/tarball:/t:ro" debian:trixie sh /t/test-install.sh /pkg
#
# Prints "ok" for each scenario and exits non-zero at the first failure.
set -eu

pkg=${1:?usage: test-install.sh <unpacked tarball folder>}
link=/etc/systemd/system/multi-user.target.wants/koppel.service
calls=/tmp/calls
active=/tmp/active

mkdir -p /usr/local/bin
# Stands in for systemctl: enabled means the link exists, active means the flag file does.
cat > /usr/local/bin/systemctl <<'S'
#!/bin/sh
echo "$*" >> /tmp/calls
link=/etc/systemd/system/multi-user.target.wants/koppel.service
args=""
for a in "$@"; do case "$a" in --*) ;; *) args="$args $a" ;; esac; done
set -- $args
case "$1" in
  enable) mkdir -p "$(dirname "$link")"; touch "$link" ;;
  disable) rm -f "$link"; rm -f /tmp/active ;;
  start) touch /tmp/active ;;
  try-restart) : ;;
  is-enabled) [ -e "$link" ] ;;
esac
S
chmod +x /usr/local/bin/systemctl
printf '#!/bin/sh\nexit 0\n' > /usr/local/bin/modprobe
chmod +x /usr/local/bin/modprobe

fail() { echo "FAIL: $1" >&2; echo "calls:" >&2; cat "$calls" >&2 2>/dev/null || true; exit 1; }
reset() { : > "$calls"; }
has() { grep -qx "$1" "$calls" || fail "$2 (no call '$1')"; }
hasnt() { if grep -qx "$1" "$calls"; then fail "$2 (unexpected call '$1')"; fi; }

# --- first install, systemd running
mkdir -p /run/systemd/system
reset
sh "$pkg/install.sh" > /tmp/out
[ -x /usr/lib/koppel/koppel ] || fail "binary not installed"
[ "$(readlink /usr/local/bin/koppel)" = /usr/lib/koppel/koppel ] || fail "link missing"
[ -f /etc/systemd/system/koppel.service ] || fail "unit missing"
[ -f /etc/modules-load.d/koppel.conf ] || fail "module list missing"
[ -f /usr/local/share/man/man1/koppel.1.gz ] || fail "man page missing"
[ -f /usr/share/doc/koppel/LICENSE ] && [ -f /usr/share/doc/koppel/THIRD-PARTY-NOTICES.txt ] || fail "docs missing"
has "enable koppel.service" "first install enables"
has "start koppel.service" "first install starts"
grep -q "Open https://" /tmp/out || fail "no first-run message"
echo "ok first install"

# --- upgrade: enabled unit stays enabled and is restarted
reset
sh "$pkg/install.sh" > /tmp/out
hasnt "enable koppel.service" "upgrade does not enable again"
hasnt "start koppel.service" "upgrade does not start"
has "try-restart koppel.service" "upgrade restarts a running service"
[ -e "$link" ] || fail "unit lost its enabled link"
echo "ok upgrade"

# --- upgrade of a disabled unit stays disabled
rm -f "$link"
reset
sh "$pkg/install.sh" > /tmp/out
hasnt "enable koppel.service" "disabled unit stays disabled"
[ ! -e "$link" ] || fail "disabled unit got enabled"
echo "ok upgrade of a disabled unit"

# --- removal keeps the data, purge deletes it
mkdir -p /var/lib/koppel; touch /var/lib/koppel/keep
sh "$pkg/uninstall.sh" > /tmp/out
[ ! -e /usr/lib/koppel ] && [ ! -e /etc/systemd/system/koppel.service ] && [ ! -e /usr/local/bin/koppel ] || fail "files left after uninstall"
[ ! -e /etc/modules-load.d/koppel.conf ] && [ ! -e /usr/local/share/man/man1/koppel.1.gz ] || fail "files left after uninstall"
[ -f /var/lib/koppel/keep ] || fail "uninstall deleted the data"
echo "ok uninstall"
sh "$pkg/install.sh" > /tmp/out
sh "$pkg/uninstall.sh" --purge > /tmp/out
[ ! -e /var/lib/koppel ] || fail "purge kept the data"
echo "ok purge"

# --- a .deb already installed: refuse
mkdir -p /tmp/fakebin
printf '#!/bin/sh\nexit 0\n' > /tmp/fakebin/dpkg
chmod +x /tmp/fakebin/dpkg
if PATH="/tmp/fakebin:$PATH" sh "$pkg/install.sh" > /tmp/out 2>&1; then fail "installed beside a .deb"; fi
grep -q "installed as a .deb" /tmp/out || fail "no message about the .deb"
[ ! -e /usr/lib/koppel ] || fail "files installed beside a .deb"
echo "ok refuses beside a .deb"

# --- no systemd: files in place, no service calls
rmdir /run/systemd/system
reset
sh "$pkg/install.sh" > /tmp/out
[ -x /usr/lib/koppel/koppel ] || fail "binary not installed without systemd"
[ ! -s "$calls" ] || fail "systemctl called without systemd"
grep -q "does not run systemd" /tmp/out || fail "no message without systemd"
echo "ok without systemd"
