#!/bin/bash
# Builds the Koppel image here and runs it on another machine over SSH. No registry needed.
#   bash src/installer/docker/deploy.sh user@thin-client
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

# The image name and tag come from the deploy compose, the file that runs on the target, so what is built and
# saved here is exactly what that compose looks for (and, with its pull_policy, never pulls over).
image="$(docker compose -f "$here/docker-compose.yml" config --images)"

echo "Building $image..."
docker build -f "$src/koppel/Dockerfile" -t "$image" "$src"

echo "Copying $image to $target..."
docker save "$image" | gzip | ssh "$target" 'gunzip | docker load'

echo "Starting Koppel on $target..."
ssh "$target" 'mkdir -p ~/koppel'
scp "$here/docker-compose.yml" "$target:~/koppel/docker-compose.yml"
ssh "$target" 'cd ~/koppel && docker compose up -d && docker compose ps'

host="${target#*@}"
echo "Koppel is running. Open https://$host:5240/"
