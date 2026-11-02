#!/bin/bash
# Builds the Koppel image here and runs it on another machine over SSH. No registry needed.
#   bash src/Koppel/deploy/deploy.sh user@thin-client
#
# The target needs Docker and the USB/IP modules: run prepare-host.sh there once first.
# The image is built for this machine's architecture; the script stops if the target differs.
set -euo pipefail

target="${1:?Usage: deploy.sh user@host}"
here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/../.." && pwd)"

local_arch="$(docker version --format '{{.Server.Arch}}')"
remote_arch="$(ssh "$target" 'docker version --format "{{.Server.Arch}}"')"
if [ "$local_arch" != "$remote_arch" ]; then
  echo "This machine builds $local_arch images but $target runs $remote_arch. Build there instead, or use docker buildx." >&2
  exit 1
fi

echo "Building the image..."
docker compose -f "$src/Koppel/docker-compose.yml" build

echo "Copying the image to $target..."
docker save matumi/koppel:latest | gzip | ssh "$target" 'gunzip | docker load'

echo "Starting Koppel on $target..."
ssh "$target" 'mkdir -p ~/koppel'
scp "$here/docker-compose.yml" "$target:~/koppel/docker-compose.yml"
ssh "$target" 'cd ~/koppel && echo "KOPPEL_HOSTNAME=$(hostname)" > .env && docker compose up -d && docker compose ps'

host="${target#*@}"
echo "Koppel is running. Open http://$host:5240/"
