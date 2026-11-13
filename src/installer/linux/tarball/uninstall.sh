#!/bin/sh
# Removes what install.sh installed. Config and keys in /var/lib/koppel stay, so a reinstall picks them up
# again; sudo ./uninstall.sh --purge deletes them too.
set -eu

if [ "$(id -u)" -ne 0 ]; then
  echo "Run this as root: sudo ./uninstall.sh" >&2
  exit 1
fi

purge=no
case "${1-}" in
  '') ;;
  --purge) purge=yes ;;
  *) echo "Usage: sudo ./uninstall.sh [--purge]" >&2; exit 1 ;;
esac

if [ -d /run/systemd/system ]; then
  systemctl disable --now koppel.service 2>/dev/null || true
fi

rm -f /etc/systemd/system/koppel.service /etc/modules-load.d/koppel.conf /usr/local/bin/koppel \
  /usr/local/share/man/man1/koppel.1.gz
rm -rf /usr/lib/koppel /usr/share/doc/koppel

if [ -d /run/systemd/system ]; then
  systemctl daemon-reload || true
fi

if [ "$purge" = yes ]; then
  rm -rf /var/lib/koppel
  echo "Koppel is removed, and its config and keys with it."
else
  echo "Koppel is removed. Its config and keys are still in /var/lib/koppel (uninstall.sh --purge deletes them)."
fi
