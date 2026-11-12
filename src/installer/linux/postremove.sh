#!/bin/sh
# Runs after the .deb is removed. Config and keys stay on a remove, so a reinstall picks them up again; only a
# purge (apt purge koppel) deletes them.
set -e

if [ "$1" = "purge" ]; then
  rm -rf /var/lib/koppel
fi

if [ -d /run/systemd/system ]; then
  systemctl daemon-reload || true
fi

if [ "$1" = "remove" ] && command -v deb-systemd-helper >/dev/null 2>&1; then
  deb-systemd-helper mask koppel.service >/dev/null || true
fi

if [ "$1" = "purge" ] && command -v deb-systemd-helper >/dev/null 2>&1; then
  # Forgets the unit's links and state, so a later install starts from scratch.
  deb-systemd-helper purge koppel.service >/dev/null || true
  deb-systemd-helper unmask koppel.service >/dev/null || true
fi
