# Receiver modernization roadmap

This document is the execution map for the OpenWebRX/NyxScope-inspired
PulseScope receiver. Maturity claims remain in `release/acceptance-matrix.json`;
this file explains sequence and product intent.

## Product model

- **Receiver** is waterfall-first: choose a band, move the hardware window,
  click a signal, listen, and save it.
- **Monitor** is the dense multi-VFO view: current channels, signal state,
  decoder activity, and health.
- A **hardware window** is one physical capture span shared by LAN clients.
- A **listener session** is one browser's viewport and selected VFO. It never
  silently retunes the shared device.
- **Profiles** persist hardware-window settings. **Bookmarks** persist channels.

## Implemented foundation

- Responsive DPR-aware spectrum and worker waterfall with a larger useful
  history on desktop and mobile.
- Explicit click/drag center mode, wheel panning, numeric center entry, usable
  bandwidth display, and capability-derived sample-rate choices.
- Separate Receiver and Monitor workspaces.
- SQLite-backed profiles and bookmarks plus configured bandplan API.
- Revisioned per-browser listener sessions and an explicit hardware-window API.
- Profile application with rollback on rejected device settings.
- Versioned health, spectrum, receiver, session, decoder, recording, and media
  contracts; truthful feature maturity is generated from the acceptance matrix.
- Canonical contributor and AI-agent handoff documentation.

## Cutover sequence

1. Add API conformance tests for profiles, bookmarks, listener concurrency, and
   rollback; add desktop and phone browser flows for both workspaces.
2. Add viewport-specific FFT resampling without changing physical capture,
   persistent client session expiry, and an operator-confirmed shared retune
   countdown.
3. Complete OpenWebRX-style station labels, bandplan overlays, scan lists,
   scheduled background services, and decoder suggestions.
4. Add normalized geo entities and an offline-capable map for ADS-B, AIS,
   radiosondes, APRS, and satellite passes. Never infer positions from text.
5. Add timed/manual IQ and audio recording with storage quotas, metadata,
   playback, and export.
6. Promote decoders only after recorded-IQ end-to-end fixtures. Encrypted voice
   is identified and logged, never decrypted.
7. Run RSP1B canary, mobile, recovery, two-hour audio, eight-hour hardware, and
   final 72-hour soak gates before replacing port 8080.

## Non-negotiable gates

- No frozen spectrum may be labeled live.
- No mock, process-only, or unit-only decoder result may be labeled available.
- Unsupported gains, bandwidths, rates, antennas, or settings return an error;
  they are never silently substituted.
- A failed profile or retune must preserve or restore the prior hardware state.
- Production cutover always preserves the previous image and data for rollback.
