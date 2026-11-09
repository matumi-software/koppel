#!/usr/bin/env bash
# Runs the contract tests against the Rust build: builds `koppel`, starts the targets the tests need,
# each from a fresh temporary data folder on its own port, runs the tests, then stops them and removes
# the folders, whether the tests passed or not.
#
#   bash src/koppel-contract/run-against-rust.sh [cargo test arguments...]
#
# KOPPEL_CONTRACT_PORT moves the seven ports (16240 up, never 5240, where an installed Koppel listens),
# KOPPEL_CONTRACT_RELEASE=1 builds a release build (which needs the UI built first), and extra
# arguments go to `cargo test`, such as `--test push` to run one file. Needs cargo and curl; on
# Windows, run it from Git Bash.
#
# The targets push over Server-Sent Events and have no Skakel yet (#85), so the tests that need
# Skakel's endpoints say they were skipped (KOPPEL_CONTRACT_SKAKEL=0).
#
# On Windows every target listens on 127.0.0.1 only, so no firewall prompt comes up, and the
# Ui:AllowFrom test is skipped: it needs a caller that is not this machine's loopback. Elsewhere the
# targets listen on every address and that test reaches its target at this machine's own address.

set -euo pipefail

port="${KOPPEL_CONTRACT_PORT:-16240}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
src="$here/.."

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) windows=1 ;;
  *) windows=0 ;;
esac

profile=debug
build=(cargo build -p koppel)
if [ "${KOPPEL_CONTRACT_RELEASE:-0}" = "1" ]; then
  profile=release
  build+=(--release)
fi
(cd "$src" && "${build[@]}")
exe="$src/target/$profile/koppel"
[ "$windows" = "1" ] && exe="$exe.exe"

# A path as the program reads it: a Windows path on Windows.
native() {
  if [ "$windows" = "1" ]; then cygpath -w "$1"; else printf '%s\n' "$1"; fi
}

for p in $(seq "$port" $((port + 6))); do
  if [ "$(curl -sk -o /dev/null -w '%{http_code}' --max-time 2 "https://127.0.0.1:$p/healthz" || true)" != "000" ]; then
    echo "Port $p is taken. Set KOPPEL_CONTRACT_PORT to move the targets." >&2
    exit 1
  fi
done

data="$(mktemp -d)"
pids=()
cleanup() {
  for pid in "${pids[@]}"; do
    kill "$pid" >/dev/null 2>&1 || true
  done
  for pid in "${pids[@]}"; do
    wait "$pid" >/dev/null 2>&1 || true
  done
  rm -rf "$data"
}
trap cleanup EXIT

address=()
[ "$windows" = "1" ] && address=(Ports__Address=127.0.0.1)

# start NAME UI_PORT [NAME=VALUE settings...]
start() {
  local name="$1" ui_port="$2"
  shift 2
  mkdir -p "$data/$name"
  env Paths__Config="$(native "$data/$name/config")" Paths__Keys="$(native "$data/$name/keys")" \
    Ports__Ui="$ui_port" "${address[@]}" "$@" "$exe" >"$data/$name.log" 2>&1 &
  pids+=($!)
}

# The expired one first: its setup window of one minute runs while the others start.
start expired $((port + 3)) Ui__SetupWindowMinutes=1
start main $((port + 0)) RateLimits__AuthPerMinute=1000
start client $((port + 1)) RateLimits__AuthPerMinute=1000
start fresh $((port + 2))
start guarded $((port + 4)) Ui__AllowFrom=local Ports__Network=$((port + 5))
start limits $((port + 6))

for p in $(seq "$port" $((port + 6))); do
  code=000
  for _ in $(seq 1 120); do
    code="$(curl -sk -o /dev/null -w '%{http_code}' --max-time 2 "https://127.0.0.1:$p/healthz" || true)"
    [ "$code" != "000" ] && break
    sleep 0.5
  done
  if [ "$code" = "000" ]; then
    echo "Koppel on port $p did not start." >&2
    tail -20 "$data"/*.log >&2 || true
    exit 1
  fi
done

export KOPPEL_CONTRACT_URL="https://localhost:$((port + 0))"
export KOPPEL_CONTRACT_CLIENT_URL="https://localhost:$((port + 1))"
export KOPPEL_CONTRACT_PEER_URL="https://localhost:$((port + 0))"
export KOPPEL_CONTRACT_FRESH_URL="https://localhost:$((port + 2))"
export KOPPEL_CONTRACT_EXPIRED_URL="https://localhost:$((port + 3))"
export KOPPEL_CONTRACT_LIMITS_URL="https://localhost:$((port + 6))"
export KOPPEL_CONTRACT_PUSH=sse
export KOPPEL_CONTRACT_SKAKEL=0

# Ui:AllowFrom=local turns away any caller that is not loopback, so the tests reach the guarded
# target at this machine's own address, which a connection from here comes from too.
own=""
if [ "$windows" = "0" ]; then
  own="$(hostname -I 2>/dev/null | awk '{print $1}' || true)"
fi
if [ -n "$own" ]; then
  export KOPPEL_CONTRACT_GUARDED_URL="https://$own:$((port + 4))"
  export KOPPEL_CONTRACT_GUARDED_NETWORK_URL="https://localhost:$((port + 5))"
else
  echo "The Ui:AllowFrom test is skipped: no address of this machine to reach the guarded target at." >&2
fi

cd "$src"
# One at a time in name order, so a run reads the same each time. The lock would serialize them anyway.
export RUST_TEST_THREADS=1
cargo test -p koppel-contract "$@"
