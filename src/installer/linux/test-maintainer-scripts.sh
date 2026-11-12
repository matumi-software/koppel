#!/bin/sh
# Runs postinstall.sh, preremove.sh and postremove.sh against Debian's own deb-systemd-helper and
# deb-systemd-invoke, with a stand-in for systemctl, and checks what an install, an upgrade, an upgrade
# of a disabled or stopped service, a removal and a purge do. No systemd runs; the stand-in keeps the
# unit's enabled link and an "active" flag, and logs each call. Run it in a Debian container, from the repo:
#
#   docker run --rm -v "$PWD/src/installer/linux:/linux:ro" debian:trixie sh /linux/test-maintainer-scripts.sh
#
# Prints "ok" for each scenario and exits non-zero at the first failure.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
unit=koppel.service
link=/etc/systemd/system/multi-user.target.wants/$unit
calls=/tmp/calls
active=/tmp/active

mkdir -p /run/systemd/system /usr/lib/systemd/system /var/lib/koppel /usr/local/bin
cp "$here/koppel.service" /usr/lib/systemd/system/$unit
# Docker's Debian images ship a policy-rc.d that forbids starting services; it comes back at the end.
rm -f /usr/sbin/policy-rc.d

# Stands in for systemctl: enabled means the link exists, active means the flag file does.
cat > /usr/local/bin/systemctl <<'S'
#!/bin/sh
echo "$*" >> /tmp/calls
link=/etc/systemd/system/multi-user.target.wants/koppel.service
args=""
for a in "$@"; do case "$a" in --*) ;; *) args="$args $a" ;; esac; done
set -- $args
case "$1" in
  is-enabled) if [ -L "$link" ]; then echo enabled; else echo disabled; exit 1; fi ;;
  is-active) [ -f /tmp/active ] ;;
  enable) mkdir -p "$(dirname "$link")"; ln -sf /usr/lib/systemd/system/koppel.service "$link" ;;
  disable) rm -f "$link" ;;
  start|restart) touch /tmp/active ;;
  try-restart) [ -f /tmp/active ] && echo "restarted" >> /tmp/calls || true ;;
  stop) rm -f /tmp/active ;;
  *) ;;
esac
S
chmod +x /usr/local/bin/systemctl

fail() { echo "FAIL: $*" >&2; echo "calls:" >&2; cat "$calls" >&2 2>/dev/null || true; exit 1; }
reset() {
  rm -f "$calls" "$active" "$link"
  rm -rf /var/lib/systemd/deb-systemd-helper-enabled /var/lib/systemd/deb-systemd-helper-masked
  rm -f /etc/systemd/system/koppel.service
  : > "$calls"
}
ran() { grep -q "$1" "$calls"; }
# dpkg sets these, and deb-systemd-helper refuses to run without them.
export DPKG_MAINTSCRIPT_PACKAGE=koppel
run() {
  script=$1; shift
  case $script in postinstall.sh) name=postinst ;; preremove.sh) name=prerm ;; *) name=postrm ;; esac
  DPKG_MAINTSCRIPT_NAME=$name sh "$here/$script" "$@" > /tmp/out 2>&1 || { cat /tmp/out >&2; fail "$script $* exited non-zero"; }
}

reset
run postinstall.sh configure
[ -L "$link" ] || fail "a first install did not enable the unit"
[ -f "$active" ] || fail "a first install did not start the service"
ran "restarted" && fail "a first install restarted"
grep -q "Koppel is running" /tmp/out || fail "a first install did not say how to open Koppel"
echo "ok first install enables and starts"

touch "$active"; : > "$calls"
run postinstall.sh configure 0.2.0
[ -L "$link" ] || fail "an upgrade disabled the unit"
ran "try-restart" || fail "an upgrade of a running service did not try-restart"
ran "restarted" || fail "an upgrade did not restart the running service"
ran " restart koppel" && fail "an upgrade used restart, which starts a stopped service"
echo "ok upgrade restarts a running service"

rm -f "$active"; : > "$calls"
run postinstall.sh configure 0.2.0
[ -f "$active" ] && fail "an upgrade started a stopped service"
[ -L "$link" ] || fail "an upgrade of a stopped service disabled the unit"
echo "ok upgrade leaves a stopped service stopped"

# The administrator disables the unit after the helper recorded it: an upgrade keeps it disabled.
systemctl disable $unit; rm -f "$active"; : > "$calls"
run postinstall.sh configure 0.2.1
[ -L "$link" ] && fail "an upgrade enabled a unit the administrator disabled"
[ -f "$active" ] && fail "an upgrade started a disabled, stopped service"
echo "ok upgrade keeps a disabled unit disabled"

# 0.2.0 enabled the unit with systemctl and left no helper record; an administrator disabled it.
reset
run postinstall.sh configure 0.2.0
[ -L "$link" ] && fail "an upgrade from a package with no helper record enabled a disabled unit"
echo "ok upgrade from a package with no helper record keeps a disabled unit disabled"

# The same, still enabled: it is enabled, and recorded for the next upgrade.
reset
systemctl enable $unit; touch "$active"; : > "$calls"
run postinstall.sh configure 0.2.0
[ -L "$link" ] || fail "an upgrade of an enabled unit with no helper record disabled it"
ran "restarted" || fail "that upgrade did not restart the running service"
echo "ok upgrade from a package with no helper record keeps an enabled unit enabled"

# Remove stops the service and masks the unit; purge forgets everything and deletes the data.
run preremove.sh remove
[ -f "$active" ] && fail "remove did not stop the service"
run postremove.sh remove
[ -L /etc/systemd/system/$unit ] || fail "remove did not mask the unit"
echo "ok remove stops and masks"

touch "$active"; : > "$calls"
run preremove.sh upgrade 0.2.1
[ -f "$active" ] || fail "preremove stopped the service on an upgrade"
echo "ok an upgrade's preremove leaves the service running"

echo data > /var/lib/koppel/file
run postremove.sh purge
[ -e /var/lib/koppel ] && fail "purge kept /var/lib/koppel"
[ -e /etc/systemd/system/$unit ] && fail "purge left the mask"
echo "ok purge deletes the data and the unit's state"

# A policy-rc.d that says no (a chroot, an image build): the unit is enabled, nothing starts, nothing fails.
reset
printf '#!/bin/sh
exit 101
' > /usr/sbin/policy-rc.d
chmod +x /usr/sbin/policy-rc.d
run postinstall.sh configure
rm -f /usr/sbin/policy-rc.d
[ -L "$link" ] || fail "a policy-rc.d refusal stopped the unit from being enabled"
[ -f "$active" ] && fail "a policy-rc.d refusal did not stop the service from starting"
echo "ok policy-rc.d is respected"
