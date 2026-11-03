#!/bin/sh
# Runs after the .deb is installed or upgraded ("configure"). Loads the USB/IP modules now (the package's
# modules-load.d file loads them at boot), then enables the service and (re)starts it so an upgrade runs the
# new version.
set -e

if [ "$1" != "configure" ]; then
  exit 0
fi

for module in usbip-host vhci-hcd; do
  if ! modprobe "$module" 2>/dev/null; then
    echo "Koppel: could not load the kernel module $module." >&2
    echo "On an Ubuntu cloud or minimal kernel the USB/IP modules are in a separate package:" >&2
    echo "  sudo apt-get install linux-modules-extra-$(uname -r)" >&2
  fi
done

if [ -d /run/systemd/system ]; then
  systemctl daemon-reload
  systemctl enable koppel.service
  # A service that will not start is reported, not a failed install: the package is in place and the log
  # says why.
  if systemctl restart koppel.service; then
    echo "Koppel is running. Open https://$(hostname):5240/"
    echo "On a new install, set the password there within 10 minutes; whoever opens it first sets it."
    echo "After that, sudo systemctl restart koppel opens setup again."
    echo "The browser warns about Koppel's own certificate the first time."
  else
    echo "Koppel did not start. See sudo journalctl -u koppel." >&2
  fi
fi
