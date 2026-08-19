# Contributing to PulseScope

Start with `AGENTS.md`, then inspect the acceptance component and relevant architecture boundary before editing.

## Workflow

1. Create or reuse a feature branch and inspect `git status`.
2. Identify acceptance-matrix components affected and their open gates.
3. Add a deterministic fixture or contract test before promoting behavior.
4. Implement through the owning subsystem instead of bypassing contracts in a route or UI page.
5. Exercise explicit error, stale-state, reconnect, and unsupported-capability behavior.
6. Update API/architecture/operations documentation and structured evidence together.
7. Run the required checks from `AGENTS.md` and hand off with the supplied template.

## Public APIs and storage

Use `/api/v2` for new contracts. Commands require a unique command ID and expected revision; events require sequence and capture time. Do not add success responses for unavailable work. Add an append-only numbered SQLite migration for persistent fields and test both a clean database and upgrade from the previous schema.

## DSP and hardware

Use recorded IQ for deterministic testing and name the fixture provenance and expected result. Physical evidence must include device, driver/API version, sample rate, duration, date, counters, and result. Mock input is development-only. Controls are generated from reported capabilities, and a failed readback is a failed command.

## UI

Receiver behavior must remain usable at 360 CSS pixels and with keyboard navigation. All controls need labels, touch targets, loading/error/recovery states, and capability-based visibility. Spectrum and media work must keep queues bounded and avoid main-thread DSP or unbounded animation/network loops.

## Decoders and licensing

Integrate mature decoders out of process using manifests and typed normalized events. Pin versions and checksums, document licenses and redistribution limits, enforce resources, and add a recorded-IQ end-to-end fixture. Do not commit downloaded proprietary packages or accept licenses on a user's behalf.

## Deployment evidence

Synthetic canaries may run beside production only with physical selection disabled. A physical SDR has one owner; stop production for the shortest controlled handoff, validate readiness and media, and restore production immediately on failure. Never replace port 8080 before the declared browser, hardware, recovery, and soak gates pass.
