# PulseScope Appliance Deployment

## Build once, move the image

The receiver laptop does not need to compile PulseScope. Build on a faster Linux Docker host, transfer the immutable image, then run the same Compose file:

```sh
docker build -t pulsescope:VERSION .
docker save pulsescope:VERSION | gzip > pulsescope-VERSION.tar.gz
scp pulsescope-VERSION.tar.gz receiver:/tmp/
ssh receiver 'gzip -dc /tmp/pulsescope-VERSION.tar.gz | docker load'
```

Use a commit hash for `VERSION`. Image transfer does not include configuration, proprietary drivers, calibration, or recordings; those remain in named volumes on the receiver.

## First-run preflight

```sh
docker compose --profile preflight run --rm preflight
```

Preflight checks the USB mount and permissions, shared memory, free data storage, host receive-buffer ceiling, SoapySDR installation/modules, detected radios, and persistent driver volume. A failure blocks acceptance; a warning explains recommended tuning without claiming the receiver is unusable.

The SDRplay API remains license-controlled. PulseScope does not download or accept it automatically. Install only a pinned vendor package after reviewing its license, version, source, checksum, and restart impact. The driver volume is read-only in the long-running receiver.

## Canary and cutover

Run a candidate with a separate name, port, and data volume:

```sh
docker run -d --name pulsescope-canary --restart=no \
  -p 18080:8765 \
  -e PULSESCOPE_BIND=0.0.0.0 -e PULSESCOPE_PORT=8765 \
  -e PULSESCOPE_DATA_DIR=/var/lib/pulsescope -e PULSESCOPE_UI_DIR=/app/ui \
  -e PULSESCOPE_AUDIO_OUTPUT=0 \
  -v pulsescope-canary-data:/var/lib/pulsescope \
  pulsescope:VERSION
```

Synthetic/browser validation can run concurrently. A physical SDR has one owner, so live-RF acceptance requires a controlled handoff: stop production, attach the same restricted USB/driver mounts to the canary, run the hardware gates, stop canary, and immediately restart production if any gate fails. Do not point a canary at the production data volume.

Cut over port 8080 only after `/api/health/ready` reports advancing physical samples, `/api/v2/system/health` reports fresh FFT flow, browser tuning/audio tests pass, and the required soak finishes. Preserve the previous image tag and never reuse it for a new build.

## Backup and restore

```sh
sh deploy/backup.sh /safe/path/pulsescope-backup.tar.gz
sh deploy/restore.sh /safe/path/pulsescope-backup.tar.gz
```

Restore requires typing `RESTORE`, stops only the PulseScope service, replaces the exact PulseScope data volume, and restarts it. Back up the external proprietary-driver volume separately under the applicable vendor license.

## Trusted LAN and remote access

The default appliance is intentionally open on a trusted LAN. Set `PULSESCOPE_AUTH_TOKEN` for bearer-token access. Configure `PULSESCOPE_TLS_CERT` and `PULSESCOPE_TLS_KEY` or place the appliance behind a maintained TLS reverse proxy before exposing it outside the LAN. Never forward port 8080 directly to the public internet.
