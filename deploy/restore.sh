#!/bin/sh
set -eu

archive=${1:-}
if [ -z "$archive" ] || [ ! -f "$archive" ]; then
  printf 'usage: %s /absolute/path/to/pulsescope-backup.tar.gz\n' "$0" >&2
  exit 2
fi
case "$archive" in /*) ;; *) archive="$(cd "$(dirname "$archive")" && pwd)/$(basename "$archive")" ;; esac
volume=${PULSESCOPE_DATA_VOLUME:-}
if [ -z "$volume" ] && docker inspect pulsescope >/dev/null 2>&1; then
  volume=$(docker inspect -f '{{range .Mounts}}{{if eq .Destination "/var/lib/pulsescope"}}{{.Name}}{{end}}{{end}}' pulsescope)
fi
if [ -z "$volume" ]; then
  project=${COMPOSE_PROJECT_NAME:-$(basename "$(pwd)")}
  volume="${project}_pulsescope-data"
fi
docker volume inspect "$volume" >/dev/null 2>&1 || { printf 'PulseScope data volume %s does not exist\n' "$volume" >&2; exit 1; }
printf 'Restore target: Docker volume %s\nArchive: %s\n' "$volume" "$archive"
printf 'Type RESTORE to replace the volume contents: '
read -r confirmation
[ "$confirmation" = RESTORE ] || { printf 'Restore cancelled.\n'; exit 1; }
docker compose stop pulsescope
docker run --rm -v "$volume:/target" -v "$(dirname "$archive"):/backup:ro" alpine:3.20 \
  sh -eu -c 'find /target -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +; tar -C /target -xzf "/backup/$1"' sh "$(basename "$archive")"
docker compose up -d pulsescope
printf 'Restore complete. Verify /api/health/ready before use.\n'
