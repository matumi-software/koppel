#!/usr/bin/env bash
# Runs the contract tests against Koppel 0.1.2 in Docker, then removes every container, volume and
# network it started, whether the tests passed or not.
#
#   bash src/koppel-contract/run-against-0.1.2.sh [cargo test arguments...]
#
# KOPPEL_IMAGE runs another image, KOPPEL_CONTRACT_PORT moves the seven host ports (15240 up).
# Extra arguments go to `cargo test`, such as a test name to run only that one. Needs Docker, curl and
# cargo; on Windows, run it from Git Bash.
#
# The targets publish their ports on 127.0.0.1 only and never touch port 5240 on the host, where an
# installed Koppel listens. Each one starts on a fresh named volume.

set -euo pipefail

image="${KOPPEL_IMAGE:-matumisoftware/koppel:0.1.2}"
port="${KOPPEL_CONTRACT_PORT:-15240}"
prefix="koppel-contract"
network="$prefix-net"
names=(main client fresh expired guarded limits)

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cleanup() {
  for name in "${names[@]}"; do
    docker rm -f "$prefix-$name" >/dev/null 2>&1 || true
    docker volume rm "$prefix-$name" >/dev/null 2>&1 || true
  done
  docker network rm "$network" >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup

docker network create "$network" >/dev/null

# start NAME HOST_PORT [docker run options...]
start() {
  local name="$1" host_port="$2"
  shift 2
  # MSYS_NO_PATHCONV: Git Bash would otherwise turn /koppel into a Windows path.
  MSYS_NO_PATHCONV=1 docker run -d --name "$prefix-$name" --hostname "$prefix-$name" --network "$network" \
    -v "$prefix-$name:/koppel" -p "127.0.0.1:$host_port:5240" "$@" "$image" >/dev/null
}

# The expired one first: its setup window of one minute runs while the others start.
start expired $((port + 3)) -e Ui__SetupWindowMinutes=1
start main $((port + 0)) -e RateLimits__AuthPerMinute=1000
start client $((port + 1)) -e RateLimits__AuthPerMinute=1000
start fresh $((port + 2))
start guarded $((port + 4)) -p "127.0.0.1:$((port + 5)):5241" -e Ui__AllowFrom=local -e Ports__Network=5241
start limits $((port + 6))

# Ready once each answers anything over HTTPS (the guarded one answers its UI with 403).
for host_port in $((port + 0)) $((port + 1)) $((port + 2)) $((port + 3)) $((port + 4)) $((port + 5)) $((port + 6)); do
  for _ in $(seq 1 120); do
    code="$(curl -sk -o /dev/null -w '%{http_code}' "https://127.0.0.1:$host_port/healthz" || true)"
    if [ "$code" != "000" ]; then
      break
    fi
    sleep 0.5
  done
  if [ "$code" = "000" ]; then
    echo "Koppel on port $host_port did not start." >&2
    docker logs "$prefix-main" 2>&1 | tail -20 >&2
    exit 1
  fi
done

export KOPPEL_CONTRACT_URL="https://localhost:$((port + 0))"
export KOPPEL_CONTRACT_CLIENT_URL="https://localhost:$((port + 1))"
export KOPPEL_CONTRACT_PEER_URL="https://$prefix-main:5240"
export KOPPEL_CONTRACT_FRESH_URL="https://localhost:$((port + 2))"
export KOPPEL_CONTRACT_EXPIRED_URL="https://localhost:$((port + 3))"
export KOPPEL_CONTRACT_GUARDED_URL="https://localhost:$((port + 4))"
export KOPPEL_CONTRACT_GUARDED_NETWORK_URL="https://localhost:$((port + 5))"
export KOPPEL_CONTRACT_LIMITS_URL="https://localhost:$((port + 6))"
export KOPPEL_CONTRACT_PUSH=signalr
export KOPPEL_CONTRACT_SKAKEL=1

cd "$here/.."
# One at a time in name order, so a run reads the same each time. The lock would serialize them anyway.
export RUST_TEST_THREADS=1
cargo test -p koppel-contract "$@"
