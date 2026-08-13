#!/bin/sh
set -eu

archive=${1:-}
if [ -z "$archive" ] || [ ! -f "$archive" ]; then
  printf 'usage: %s /absolute/path/to/pulsescope-backup.tar.gz\n' "$0" >&2
  exit 2
fi
case "$archive" in /*) ;; *) archive="$(cd "$(dirname "$archive")" && pwd)/$(basename "$archive")" ;; esac
printf 'Restore target: Docker volume pulsescope_pulsescope-data\nArchive: %s\n' "$archive"
printf 'Type RESTORE to replace the volume contents: '
read -r confirmation
[ "$confirmation" = RESTORE ] || { printf 'Restore cancelled.\n'; exit 1; }
docker compose stop pulsescope
docker run --rm -v pulsescope_pulsescope-data:/target -v "$(dirname "$archive"):/backup:ro" alpine:3.20 \
  sh -eu -c 'find /target -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +; tar -C /target -xzf "/backup/$1"' sh "$(basename "$archive")"
docker compose up -d pulsescope
printf 'Restore complete. Verify /api/health/ready before use.\n'
