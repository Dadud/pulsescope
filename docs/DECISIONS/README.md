# Architecture decisions

These decisions prevent future contributors from reopening settled boundaries without evidence.

1. The Rust server is authoritative; Svelte clients are shared across web, PWA, and wrappers.
2. A hardware window is shared while listener VFO, viewport, and audio state are independent.
3. SoapySDR is the compatibility tier; direct adapters require measured justification.
4. Spectrum transport is binary, sequenced, viewport-aware, and latest-frame-only.
5. Browser audio targets WebRTC Opus and retains timestamped PCM only as fallback.
6. Decoders are checksummed, resource-bounded isolated processes with normalized events.
7. SQLite is the appliance system of record and migrations are append-only.
8. The default security model is a trusted LAN; internet exposure requires TLS and authentication.

Changing one of these decisions requires a dated replacement ADR describing evidence, compatibility, migration, rollout, and rollback.
