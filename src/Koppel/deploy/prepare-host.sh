#!/bin/bash
# Prepares a Debian or Ubuntu machine to run Koppel. Run it once on that machine, as root:
#   sudo bash prepare-host.sh
#
# It installs Docker from Docker's own signed apt repository, loads the kernel's USB/IP modules now and
# at every boot, and lets the invoking user run docker without sudo.
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "Run this as root: sudo bash $0" >&2
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  . /etc/os-release
  case "$ID" in
    debian | ubuntu) ;;
    *) echo "This script supports Debian and Ubuntu, not '$ID'. Install Docker yourself (https://docs.docker.com/engine/install/) and run again." >&2; exit 1 ;;
  esac

  apt-get update
  apt-get install -y ca-certificates curl
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL "https://download.docker.com/linux/$ID/gpg" -o /etc/apt/keyrings/docker.asc
  chmod a+r /etc/apt/keyrings/docker.asc
  # Ubuntu derivatives report their own codename in UBUNTU_CODENAME, which is the one Docker's repository uses.
  codename="${UBUNTU_CODENAME:-$VERSION_CODENAME}"
  cat > /etc/apt/sources.list.d/docker.sources <<SOURCES
Types: deb
URIs: https://download.docker.com/linux/$ID
Suites: $codename
Components: stable
Architectures: $(dpkg --print-architecture)
Signed-By: /etc/apt/keyrings/docker.asc
SOURCES
  apt-get update
  apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
fi

# usbip-host lets this machine share devices; vhci-hcd lets it attach devices from another server.
for module in usbip-host vhci-hcd; do
  if ! modprobe "$module"; then
    echo "Could not load the kernel module $module." >&2
    echo "On an Ubuntu cloud or minimal kernel the USB/IP modules are in a separate package. Try:" >&2
    echo "  sudo apt-get install linux-modules-extra-$(uname -r)" >&2
    echo "and run this script again. If that package does not exist for this kernel, switch to the generic kernel (linux-generic)." >&2
    exit 1
  fi
done
printf 'usbip-host\nvhci-hcd\n' > /etc/modules-load.d/koppel.conf

if [ -n "${SUDO_USER:-}" ]; then
  usermod -aG docker "$SUDO_USER"
  echo "Added $SUDO_USER to the docker group. Log out and back in for it to take effect."
fi

echo "Ready. Kernel: $(uname -r), architecture: $(uname -m)."
lsmod | grep -E 'usbip_host|vhci_hcd' || true
