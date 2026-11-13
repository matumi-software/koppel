Koppel, USB over IP. The tarball for Linux machines that do not install a .deb.

The program is one static binary (musl): it needs no libraries from your system. It needs a kernel with the
USB/IP modules usbip-host and vhci-hcd, and systemd if you want the service set up for you.

  sudo ./install.sh              install, or upgrade, and start the service
  sudo ./uninstall.sh            remove it; config and keys in /var/lib/koppel stay
  sudo ./uninstall.sh --purge    remove it, config and keys too

Settings go in /etc/default/koppel as environment variables, as in koppel.1 (man koppel).
Third-party licences: THIRD-PARTY-NOTICES.txt. Project: https://github.com/matumi-software/koppel
