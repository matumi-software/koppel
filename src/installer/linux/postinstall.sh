#!/bin/sh
# Runs after the .deb is installed or upgraded ("configure"). Loads the USB/IP modules now (the package's
# modules-load.d file loads them at boot), then handles the service the way debhelper's dh_installsystemd does:
#
# - First install ($2 empty): enable the unit and start it.
# - Upgrade: leave the enabled or disabled choice alone, so a unit the administrator disabled stays disabled,
#   and restart the service only if it is running, so the new version runs. One that was stopped stays stopped.
#
# deb-systemd-helper keeps the "was it enabled" state across upgrades and purge, and deb-systemd-invoke
# respects policy-rc.d (a chroot or an image build must not start services). Both are in init-system-helpers,
# which every Debian system has; where they are missing the script falls back to systemctl.
set -e

if [ "$1" != "configure" ]; then
  exit 0
fi

UNIT=koppel.service

for module in usbip-host vhci-hcd; do
  if ! modprobe "$module" 2>/dev/null; then
    echo "Koppel: could not load the kernel module $module." >&2
    echo "On an Ubuntu cloud or minimal kernel the USB/IP modules are in a separate package:" >&2
    echo "  sudo apt-get install linux-modules-extra-$(uname -r)" >&2
  fi
done

# An upgrade enables the unit again only if it is enabled now. The helper's own record would say yes for a
# package installed before it kept one (0.2.0 enabled the unit with systemctl), whatever the administrator did.
enable=yes
if [ -n "$2" ] && [ -d /run/systemd/system ] && ! systemctl is-enabled --quiet "$UNIT" 2>/dev/null; then
  enable=no
fi

if command -v deb-systemd-helper >/dev/null 2>&1; then
  # Removes only a mask the helper made on removal. A new install has no state yet, and was-enabled then says
  # yes, so it is enabled; an upgrade enables the unit again only if it was enabled before.
  deb-systemd-helper unmask "$UNIT" >/dev/null || true
  if [ "$enable" = yes ] && deb-systemd-helper --quiet was-enabled "$UNIT"; then
    deb-systemd-helper enable "$UNIT" >/dev/null || true
  else
    # Records the new unit file's links for purge, and enables nothing.
    deb-systemd-helper update-state "$UNIT" >/dev/null || true
  fi
elif [ -d /run/systemd/system ] && [ "$enable" = yes ] && [ -z "$2" ]; then
  systemctl enable "$UNIT" || true
fi

if [ -d /run/systemd/system ]; then
  systemctl daemon-reload || true
  if [ -z "$2" ]; then
    action=start
  else
    # try-restart does nothing for a service that is not running.
    action=try-restart
  fi
  if command -v deb-systemd-invoke >/dev/null 2>&1; then
    invoke="deb-systemd-invoke $action $UNIT"
  else
    invoke="systemctl $action $UNIT"
  fi
  # A service that will not start is reported, not a failed install: the package is in place and the log
  # says why.
  if $invoke; then
    if [ -z "$2" ]; then
      echo "Koppel is running. Open https://$(hostname):5240/"
      echo "On a new install, set the password there within 10 minutes. The page asks for a setup token: sudo journalctl -u koppel shows it."
      echo "After that, sudo systemctl restart koppel opens setup again."
      echo "The browser warns about Koppel's own certificate the first time."
    fi
  else
    echo "Koppel did not start. See sudo journalctl -u koppel." >&2
  fi
fi
