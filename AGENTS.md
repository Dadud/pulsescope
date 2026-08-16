# PulseScope contributor guide

This file is the canonical entry point for human and AI contributors. Read it before changing the repository. Chat history is background only and is never a source of truth.

## Product and boundaries

PulseScope is a trusted-LAN SDR appliance: a Rust server owns hardware, bounded IQ distribution, receiver sessions, DSP, decoders, recordings, health, and SQLite state; one Svelte application provides the Receiver and Monitor workspaces and is reused by the PWA and desktop/mobile wrappers.

- `HardwareWindow` is shared: device, center, sample rate, analog/usable bandwidth, profile, revision, and health.
- `ListenerSession` is per browser: VFOs, viewport, audio, mode, filter, squelch, and decoder attachments.
- SoapySDR is the broad compatibility adapter. Direct adapters are justified by measured lifecycle or performance needs.
- External decoders remain isolated processes described by signed, checksummed manifests. No client-provided command lines.
- Encrypted traffic may be identified and logged. Only well-known public default channel keys (Meshtastic `AQ==` / simpleN, MeshCore Public) recover plaintext; private channel keys, PKI direct messages, and LoRaWAN payloads are never decrypted.

## Source-of-truth order

1. Code, migrations, generated schemas, and public API contracts.
2. `release/acceptance-matrix.json` for maturity and required gates.
3. Generated `docs/FEATURE_STATUS.md` (never edit directly).
4. `docs/ARCHITECTURE.md`, ADRs, and `docs/ULTRASDR_PRODUCT_PLAN.md`.
5. README and `NYXSCOPE_PARITY_MATRIX.md`; the latter is historical research, not release truth.
6. Issues, pull requests, and conversations.

Do not describe a feature as verified merely because a route, UI label, mock, parser unit test, or sidecar declaration exists. Use the acceptance matrix vocabulary: `planned`, `development`, `fixture_verified`, `hardware_verified`, `production`.

## Repository map

- `src-tauri/src/device.rs`, `capture.rs`: hardware capability/lifecycle and bounded IQ acquisition.
- `src-tauri/src/scanner.rs`, `demod.rs`, `audio.rs`: receiver allocation, DSP, spectrum, and media.
- `src-tauri/src/api.rs`, `state.rs`, `db.rs`: v2 contracts, session state, persistence, and recovery.
- `src-tauri/src/decoder_manifest.rs`, `sidecar.rs`: isolated decoder contracts and scheduling.
- `ui/src/routes`: Receiver, Monitor, Activity, Map, Recordings, and Settings surfaces.
- `ui/src/lib`: typed API/media clients and worker renderers.
- `src-tauri/migrations`: append-only SQLite migrations; never rewrite an applied migration.
- `release/acceptance-matrix.json`: machine-readable release truth.
- `deploy`, `docker-compose.yml`, `Dockerfile`: appliance preflight, backup, restore, canary, and packaging.

See `docs/ARCHITECTURE.md` for data flow and `docs/CONTRIBUTING.md` for the development workflow.

## Required checks

Run the smallest relevant checks while iterating, then the full gate before handing off:

```sh
pnpm install --frozen-lockfile
pnpm check
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --features headless,mock-source --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features --features headless,mock-source --lib
cargo check --manifest-path src-tauri/Cargo.toml --no-default-features --features headless,mock-source --bin pulsescope
docker build -t pulsescope:$(git rev-parse --short HEAD) .
```

`pnpm check` includes release and contributor-document verification. Hardware claims additionally require the named physical test and dated structured evidence in the acceptance matrix.

## Working rules

- Preserve unrelated user changes and inspect a dirty worktree before editing.
- Work on a feature branch. Do not force-push, merge, publish releases, replace port 8080, or push to another owner's upstream unless explicitly authorized.
- Use capability-derived controls and reject unsupported values. Never silently substitute a frequency, rate, antenna, gain, or bandwidth.
- Mock input must be visibly labelled and may satisfy only synthetic gates. It cannot satisfy physical readiness or hardware evidence.
- Bound all sample, spectrum, media, decoder, and client queues. Slow consumers drop obsolete data instead of accumulating latency.
- Readiness means advancing physical samples and FFT for hardware operation; a running process alone is insufficient.
- New public routes require API documentation and contract tests. New persistent state requires an append-only migration and migration test.
- New decoders require a manifest, pinned checksum, resource policy, health behavior, recorded-IQ fixture, normalized events, and license entry.
- Do not copy proprietary source, assets, databases, or trade dress. Follow `docs/LEGAL.md` and `docs/THIRD_PARTY_NOTICES.md`.
- Never commit passwords, LAN credentials, tokens, vendor installers, license acceptance records, recordings, or private RF data.

## Definition of done

A change is complete only when behavior, failure states, tests, documentation, and acceptance evidence agree. The normal UI must expose only usable behavior; unavailable work belongs behind an explicit Beta/Expert gate. Deployment changes must preserve the previous image and data backup until browser, hardware, recovery, and required soak gates pass.

## Handoff template

Include this in the durable issue/PR description or final handoff:

```text
Branch / last commit:
Dirty files:
Completed behavior:
Current failure or blocker:
Checks run and results:
Acceptance-matrix components affected:
Deployment/canary state and rollback image:
Next safe action:
Unverified assumptions:
```
