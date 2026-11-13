#!/bin/sh
# Installs Koppel from the release tarball, for a Linux that is not Debian based or does not use apt. Run it as
# root from the unpacked folder: sudo ./install.sh. It does what the .deb does, without a package manager:
#
# - the binary in /usr/lib/koppel with /usr/local/bin/koppel linked to it, the unit in /etc/systemd/system,
#   the module list in /etc/modules-load.d, the man page and the notices;
# - loads the USB/IP kernel modules now (the module list loads them at boot);
# - first install: enables and starts the service. An upgrade (run the new tarball's install.sh) leaves a
#   disabled service disabled and restarts the service only if it is running.
#
# Config and keys live in /var/lib/koppel, which this script never touches. uninstall.sh removes what this one
# installs. Without systemd the files are installed and the service is left for you to start: run
# /usr/lib/koppel/koppel yourself, or write the unit for your init system.
set -eu

if [ "$(id -u)" -ne 0 ]; then
  echo "Run this as root: sudo ./install.sh" >&2
  exit 1
fi

here=$(cd "$(dirname "$0")" && pwd)
UNIT=koppel.service

for file in koppel koppel.service modules-load.conf koppel.1 LICENSE THIRD-PARTY-NOTICES.txt; do
  if [ ! -f "$here/$file" ]; then
    echo "$here/$file is missing. Run install.sh from the unpacked tarball." >&2
    exit 1
  fi
done

# Two copies of the program would fight over the unit file and port 5240.
if command -v dpkg >/dev/null 2>&1 && dpkg -s koppel >/dev/null 2>&1; then
  echo "Koppel is installed as a .deb package. Update it with apt, or remove it first: sudo apt remove koppel" >&2
  exit 1
fi

# Not on the binary's final path while it runs, so an upgrade replaces it with a rename and a running
# service keeps the old file until the restart.
install -d -m 0755 /usr/lib/koppel /etc/systemd/system /etc/modules-load.d /usr/local/bin /usr/local/share/man/man1 /usr/share/doc/koppel
install -m 0755 "$here/koppel" /usr/lib/koppel/koppel.new
mv -f /usr/lib/koppel/koppel.new /usr/lib/koppel/koppel
ln -sf /usr/lib/koppel/koppel /usr/local/bin/koppel
upgrade=no
[ -f /etc/systemd/system/$UNIT ] && upgrade=yes
install -m 0644 "$here/koppel.service" /etc/systemd/system/$UNIT
install -m 0644 "$here/modules-load.conf" /etc/modules-load.d/koppel.conf
gzip -9nc "$here/koppel.1" > /usr/local/share/man/man1/koppel.1.gz
chmod 0644 /usr/local/share/man/man1/koppel.1.gz
install -m 0644 "$here/THIRD-PARTY-NOTICES.txt" /usr/share/doc/koppel/THIRD-PARTY-NOTICES.txt
install -m 0644 "$here/LICENSE" /usr/share/doc/koppel/LICENSE

for module in usbip-host vhci-hcd; do
  if ! modprobe "$module" 2>/dev/null; then
    echo "Koppel: could not load the kernel module $module." >&2
    echo "Your kernel may not ship the USB/IP modules, or ships them in a separate package (on Ubuntu cloud kernels" >&2
    echo "that is linux-modules-extra-$(uname -r))." >&2
  fi
done

if [ ! -d /run/systemd/system ]; then
  echo "Koppel is installed, but this machine does not run systemd, so the service is not set up."
  echo "Run /usr/lib/koppel/koppel as root, or write the service for your init system (see $here/koppel.service)."
  exit 0
fi

systemctl daemon-reload
if [ "$upgrade" = no ]; then
  systemctl enable $UNIT
  if systemctl start $UNIT; then
    echo "Koppel is running. Open https://$(hostname):5240/"
    echo "On a new install, set the password there within 10 minutes. The page asks for a setup token: sudo journalctl -u koppel shows it."
    echo "After that, sudo systemctl restart koppel opens setup again."
    echo "The browser warns about Koppel's own certificate the first time."
  else
    echo "Koppel did not start. See sudo journalctl -u koppel." >&2
  fi
else
  # try-restart does nothing for a service that is not running; a disabled unit stays disabled.
  systemctl try-restart $UNIT || echo "Koppel did not restart. See sudo journalctl -u koppel." >&2
  echo "Koppel is upgraded."
fi
