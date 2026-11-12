#!/bin/sh
# Runs before the .deb is removed. An upgrade keeps the service running; postinstall restarts it if it was
# running. A removal stops the service, and deb-systemd-helper disables it and keeps the state so a purge
# can clean up.
set -e

if [ "$1" = "remove" ] && [ -d /run/systemd/system ]; then
  if command -v deb-systemd-invoke >/dev/null 2>&1; then
    deb-systemd-invoke stop koppel.service || true
  else
    systemctl stop koppel.service || true
  fi
fi
