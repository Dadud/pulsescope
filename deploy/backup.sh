#!/bin/sh
set -eu

destination=${1:-./pulsescope-backup-$(date -u +%Y%m%dT%H%M%SZ).tar.gz}
case "$destination" in /*) ;; *) destination="$(pwd)/$destination" ;; esac
mkdir -p "$(dirname "$destination")"
docker run --rm -v pulsescope_pulsescope-data:/source:ro -v "$(dirname "$destination"):/backup" alpine:3.20 \
  tar -C /source -czf "/backup/$(basename "$destination")" .
printf 'Backup written to %s\n' "$destination"
