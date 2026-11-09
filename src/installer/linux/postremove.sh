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
