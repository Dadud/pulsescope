# Decoder sidecar manifest templates

These JSON files document the signed-manifest contract for SatDump and rs41mod.
They are **templates**, not installed manifests: PulseScope will not launch them
until an administrator copies them into the appliance decoder root, replaces
`executable` / `executable_sha256` with the pinned binary, and signs the payload
with a trusted Ed25519 key.

The browser never supplies an executable or command line.
