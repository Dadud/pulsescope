# Feature status and 1.0 acceptance

This document is a claim ledger, not a roadmap. The authoritative, CI-checked
evidence is [`../release/acceptance-matrix.json`](../release/acceptance-matrix.json).

| Classification | Meaning |
| --- | --- |
| Complete | Required acceptance row passed and contains reproducible evidence. |
| Experimental | Implemented, but required acceptance evidence is incomplete. |
| Dependency-gated | Requires separately installed hardware, driver, service, or decoder. |
| Unavailable | No supported end-to-end implementation; placeholders must be disabled and labelled. |

| Area | Classification | Acceptance requirement |
| --- | --- | --- |
| Mock source and core unit tests | Complete | `mock_mode` |
| Windows installers and Linux desktop | Experimental | install, uninstall, startup, path checks |
| Linux headless x86-64/ARM64 | Experimental | clean CI build and smoke |
| Authenticated API and TLS | Experimental | `authenticated_headless`, `tls` |
| Docker x86-64/ARM64 | Experimental | `docker` |
| IQ playback, recording, UDP streaming | Experimental | corresponding matrix rows |
| Native decoders | Experimental | fixture for every decoder advertised as native |
| SoapySDR hardware and external decoders | Dependency-gated | named hardware/driver or installed sidecar evidence |
| Upgrade, rollback, database migration | Experimental | preservation and compatibility fixtures |

## Release rules

1. Required CI jobs pass from a clean checkout on every supported platform.
2. Every enabled UI control invokes a documented API or local action; placeholders
   are disabled and marked unavailable. Every HTTP/WebSocket route is documented.
3. A row becomes complete only when `evidence` identifies a stable command or CI
   run. `scripts/verify_release_contract.py --release` enforces this.
4. Every artifact includes a CycloneDX SBOM and consolidated license bundle.
5. Release notes use the four classifications above. Only after all rows pass may
   maintainers set `release_ready` and create `v1.0.0` from the tested commit.
