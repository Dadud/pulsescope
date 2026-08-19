#!/usr/bin/env bash
# Deploy PulseScope to a trusted-LAN test host over SSH.
#
# Run from a machine on the same network as the appliance (not from a cloud
# agent). Credentials never belong in this repository — use SSH keys:
#
#   ssh-copy-id "${DEPLOY_USER}@${DEPLOY_HOST}"
#
# Required environment:
#   DEPLOY_HOST   e.g. 192.168.1.34
#   DEPLOY_USER   e.g. Dadud
#
# Optional:
#   DEPLOY_PORT=22
#   DEPLOY_MODE=canary|production   (default: canary on port 18080)
#   PULSESCOPE_VERSION=git tag        (default: short HEAD)
#   SKIP_BUILD=1                      reuse local pulsescope:$VERSION image
#   SKIP_PREFLIGHT=1
#   REMOTE_DOCKER=sudo docker          prefix when remote user needs sudo
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

: "${DEPLOY_HOST:?Set DEPLOY_HOST (e.g. 192.168.1.34)}"
: "${DEPLOY_USER:?Set DEPLOY_USER}"

DEPLOY_PORT="${DEPLOY_PORT:-22}"
DEPLOY_MODE="${DEPLOY_MODE:-canary}"
REMOTE_DOCKER="${REMOTE_DOCKER:-docker}"
VERSION="${PULSESCOPE_VERSION:-$(git rev-parse --short HEAD)}"
IMAGE="pulsescope:${VERSION}"
SSH=(ssh -p "$DEPLOY_PORT" -o BatchMode=yes -o ConnectTimeout=10 "${DEPLOY_USER}@${DEPLOY_HOST}")
RSYNC_SSH="ssh -p ${DEPLOY_PORT} -o BatchMode=yes"

remote() {
  "${SSH[@]}" "$@"
}

echo "==> Target ${DEPLOY_USER}@${DEPLOY_HOST}:${DEPLOY_PORT} mode=${DEPLOY_MODE} image=${IMAGE}"

echo "==> Checking SSH and Docker on remote"
remote 'command -v docker >/dev/null' || {
  echo "Remote docker not found. Install Docker or set REMOTE_DOCKER=sudo\\ docker" >&2
  exit 1
}

if [[ "${SKIP_BUILD:-0}" != "1" ]]; then
  echo "==> Building ${IMAGE} locally"
  docker build -t "$IMAGE" .
fi

ARCHIVE="/tmp/pulsescope-${VERSION}.tar.gz"
echo "==> Saving image to ${ARCHIVE}"
docker save "$IMAGE" | gzip >"$ARCHIVE"

echo "==> Transferring image ($(du -h "$ARCHIVE" | awk '{print $1}'))"
remote "mkdir -p /tmp/pulsescope-deploy"
scp -P "$DEPLOY_PORT" -o BatchMode=yes "$ARCHIVE" "${DEPLOY_USER}@${DEPLOY_HOST}:/tmp/pulsescope-deploy/image.tar.gz"

echo "==> Loading image on remote"
remote "${REMOTE_DOCKER} load -i /tmp/pulsescope-deploy/image.tar.gz && rm -f /tmp/pulsescope-deploy/image.tar.gz"

echo "==> Ensuring external driver volume exists"
remote "${REMOTE_DOCKER} volume inspect docker_sdr-drivers >/dev/null 2>&1 || ${REMOTE_DOCKER} volume create docker_sdr-drivers"

if [[ "${SKIP_PREFLIGHT:-0}" != "1" ]]; then
  echo "==> Running appliance preflight"
  remote "${REMOTE_DOCKER} run --rm \
    -e SOAPY_SDR_PLUGIN_PATH=/opt/pulsescope/drivers/sdrplay-3.15.2/modules0.8 \
    -e LD_LIBRARY_PATH=/opt/pulsescope/drivers/sdrplay-3.15.2/lib \
    -v pulsescope-data:/var/lib/pulsescope \
    -v docker_sdr-drivers:/opt/pulsescope/drivers:ro \
    -v /dev/bus/usb:/dev/bus/usb \
    -v /dev/shm:/dev/shm \
    -v /proc/sys/net/core/rmem_max:/host/rmem_max:ro \
    --device-cgroup-rule 'c 189:* rwm' \
    --entrypoint /bin/sh \
    ${IMAGE} /usr/local/lib/pulsescope/preflight.sh"
fi

if [[ "$DEPLOY_MODE" == "canary" ]]; then
  CANARY_NAME="pulsescope-canary"
  CANARY_PORT="${CANARY_PORT:-18080}"
  CANARY_DATA="pulsescope-canary-data"
  echo "==> Starting canary on port ${CANARY_PORT}"
  remote "${REMOTE_DOCKER} rm -f ${CANARY_NAME} 2>/dev/null || true"
  remote "${REMOTE_DOCKER} run -d --name ${CANARY_NAME} --restart=no \
    -p ${CANARY_PORT}:8765 \
    -e PULSESCOPE_BIND=0.0.0.0 -e PULSESCOPE_PORT=8765 \
    -e PULSESCOPE_DATA_DIR=/var/lib/pulsescope -e PULSESCOPE_UI_DIR=/app/ui \
    -e PULSESCOPE_AUDIO_OUTPUT=0 \
    -e PULSESCOPE_PREFER_PHYSICAL=0 -e PULSESCOPE_ALLOW_MOCK_READY=1 \
    -e SOAPY_SDR_PLUGIN_PATH=/opt/pulsescope/drivers/sdrplay-3.15.2/modules0.8 \
    -e LD_LIBRARY_PATH=/opt/pulsescope/drivers/sdrplay-3.15.2/lib \
    -v ${CANARY_DATA}:/var/lib/pulsescope \
    -v docker_sdr-drivers:/opt/pulsescope/drivers:ro \
    -v /dev/bus/usb:/dev/bus/usb \
    -v /dev/shm:/dev/shm \
    --device-cgroup-rule 'c 189:* rwm' \
    ${IMAGE}"
  HEALTH_URL="http://${DEPLOY_HOST}:${CANARY_PORT}/health"
else
  echo "==> Deploying production compose stack on port 8080"
  rsync -az -e "$RSYNC_SSH" docker-compose.yml deploy/ "${DEPLOY_USER}@${DEPLOY_HOST}:/tmp/pulsescope-deploy/"
  remote "cd /tmp/pulsescope-deploy && ${REMOTE_DOCKER} compose -f docker-compose.yml pull 2>/dev/null || true"
  remote "cd /tmp/pulsescope-deploy && IMAGE=${IMAGE} ${REMOTE_DOCKER} compose -f docker-compose.yml up -d --force-recreate pulsescope"
  HEALTH_URL="http://${DEPLOY_HOST}:8080/health"
fi

echo "==> Waiting for health at ${HEALTH_URL}"
for _ in $(seq 1 30); do
  if curl -fsS "$HEALTH_URL" >/tmp/pulsescope-health.json 2>/dev/null; then
    echo "Health:"
    cat /tmp/pulsescope-health.json
    echo
    echo "Ready probe:"
    curl -fsS "${HEALTH_URL%/health}/health/ready" || true
    echo
    echo "Done. Open ${HEALTH_URL%/health}/"
    rm -f "$ARCHIVE" /tmp/pulsescope-health.json
    exit 0
  fi
  sleep 2
done

echo "Health check timed out. Inspect remote logs:" >&2
remote "${REMOTE_DOCKER} logs --tail 80 pulsescope-canary 2>/dev/null || ${REMOTE_DOCKER} logs --tail 80 pulsescope"
rm -f "$ARCHIVE"
exit 1
