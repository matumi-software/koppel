#!/bin/sh
# Prepares a Debian or Ubuntu machine to run Koppel. Run it once on that machine, as root:
#   sudo sh prepare-host.sh
#
# It installs Docker, loads the kernel's USB/IP modules now and at every boot, and lets the
# invoking user run docker without sudo.
set -e

if ! command -v docker >/dev/null 2>&1; then
  apt-get update
  apt-get install -y ca-certificates curl
  curl -fsSL https://get.docker.com | sh
fi

# usbip-host lets this machine share devices; vhci-hcd lets it attach devices from another server.
modprobe usbip-host
modprobe vhci-hcd
printf 'usbip-host\nvhci-hcd\n' > /etc/modules-load.d/koppel.conf

if [ -n "$SUDO_USER" ]; then
  usermod -aG docker "$SUDO_USER"
  echo "Added $SUDO_USER to the docker group. Log out and back in for it to take effect."
fi

echo "Ready. Kernel: $(uname -r), architecture: $(uname -m)."
lsmod | grep -E 'usbip_host|vhci_hcd' || true
