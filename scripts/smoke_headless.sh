#!/usr/bin/env bash
set -euo pipefail
binary=${1:?usage: smoke_headless.sh BINARY}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/PulseScope ü space.XXXXXX")
trap 'kill ${pid:-} 2>/dev/null || true; rm -rf "$tmp"' EXIT
PULSESCOPE_DATA_DIR="$tmp/data ü" PULSESCOPE_BIND=127.0.0.1 PULSESCOPE_PORT=18765 "$binary" --server &
pid=$!
for _ in $(seq 1 30); do
  if curl -fsS http://127.0.0.1:18765/health | python3 -m json.tool >/dev/null; then exit 0; fi
  sleep 1
done
exit 1

