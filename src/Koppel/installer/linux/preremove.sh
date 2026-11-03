#!/bin/sh
# Runs before the .deb is removed. An upgrade keeps the service running; postinstall restarts it.
set -e

if [ "$1" = "remove" ] && [ -d /run/systemd/system ]; then
  systemctl disable --now koppel.service || true
fi
