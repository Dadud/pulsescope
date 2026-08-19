#!/bin/sh
set -eu

destination=${1:-./pulsescope-backup-$(date -u +%Y%m%dT%H%M%SZ).tar.gz}
case "$destination" in /*) ;; *) destination="$(pwd)/$destination" ;; esac
mkdir -p "$(dirname "$destination")"
volume=${PULSESCOPE_DATA_VOLUME:-}
if [ -z "$volume" ] && docker inspect pulsescope >/dev/null 2>&1; then
  volume=$(docker inspect -f '{{range .Mounts}}{{if eq .Destination "/var/lib/pulsescope"}}{{.Name}}{{end}}{{end}}' pulsescope)
fi
if [ -z "$volume" ]; then
  project=${COMPOSE_PROJECT_NAME:-$(basename "$(pwd)")}
  volume="${project}_pulsescope-data"
fi
docker volume inspect "$volume" >/dev/null 2>&1 || { printf 'PulseScope data volume %s does not exist\n' "$volume" >&2; exit 1; }
printf 'Backing up Docker volume %s\n' "$volume"
docker run --rm -v "$volume:/source:ro" -v "$(dirname "$destination"):/backup" alpine:3.20 \
  tar -C /source -czf "/backup/$(basename "$destination")" .
printf 'Backup written to %s\n' "$destination"
