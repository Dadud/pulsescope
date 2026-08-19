# PulseScope / UltraSDR Product Plan

## Outcome

PulseScope becomes the single maintained radio product. A headless Rust server
owns SDR hardware, DSP, receiver sessions, decoders, recordings, and persistent
state. It serves the same responsive web application used by the installable
PWA, mobile clients, and desktop wrappers.

The Linux appliance is delivered through Docker Compose. Desktop installations
use the same server binary and API without requiring Docker Desktop. Mobile
applications are clients of a PulseScope server; iOS and Android do not attempt
to host Linux containers or proprietary USB SDR drivers.

The existing UltraSDR/KA9Q checkout, images, configuration, and Docker volume
remain a rollback archive during development, but its containers stay removed.

## Product surfaces

### Docker appliance

- One supported command starts the receiver, UI, database, media service, and
  selected decoder jobs.
- A one-shot preflight discovers USB devices, validates permissions and USB
  speed, locates proprietary driver layers, and recommends a stable sample rate.
- The long-running server is unprivileged and receives only the required USB
  devices, groups, capabilities, ports, and persistent volumes.
- Compose profiles enable hardware and decoder-specific dependencies without
  bloating every installation.
- The first-run web wizard covers device choice, legal driver installation,
  antenna, region, FM de-emphasis, LAN access, storage, and decoder packs.
- Configuration, recordings, calibration, driver layers, and the SQLite
  database live in named volumes with backup, restore, and migration support.

### Web and PWA

- Svelte remains the shared UI technology.
- The server ships immutable static assets and an installable PWA manifest.
- The default receiver view is touch-first and responsive. Expert controls are
  generated from live device capabilities and remain out of the basic workflow.
- Audio uses WebRTC Opus with browser jitter handling and observable packet-loss,
  jitter, and playout statistics.
- Spectrum and waterfall use a sequence-numbered binary WebSocket protocol and
  an OffscreenCanvas/WebGL2 worker, with a bounded Canvas2D fallback.
- The PWA supports background audio where the operating system permits it,
  Media Session controls, reconnect, saved stations, and push notifications for
  configured decoder events.

### Mobile applications

- Phase one is the installable PWA, tested as a first-class mobile product.
- Phase two wraps the shared Svelte build with Capacitor for iOS and Android.
- Native plugins provide background audio, lock-screen controls, notifications,
  secure credential storage, LAN discovery, share/export, and deep links.
- Mobile apps connect to one or more PulseScope servers. Direct USB SDR hosting
  is a separate Android-only research track and is not required for launch.
- No decoder logic is duplicated in the apps; normalized events and media come
  from the server API.

### Desktop applications

- Tauri remains the thin Windows/macOS/Linux shell around the shared Svelte UI.
- Remote mode connects to a LAN PulseScope appliance.
- Local mode launches the same `pulsescope-server` binary as a managed sidecar,
  giving a no-hassle desktop installation without requiring Docker Desktop.
- The wrapper handles server lifecycle, updates, logs, firewall guidance,
  driver onboarding, tray controls, notifications, and opening the web UI.
- Local and Docker servers expose the same API and pass the same conformance
  suite. There is no separate desktop DSP implementation.

## Target architecture

```text
Hardware adapters
  SDRplay | RTL-SDR | Airspy | HackRF | IIO | UHD | Soapy | network IQ
        |
Versioned RadioDevice capability contract and hotplug supervisor
        |
Bounded, timestamped, sequence-numbered IQ sample bus
        +--> FFT/spectrum worker --> binary spectrum transport
        +--> PFB/DDC receiver allocator --> demodulators --> WebRTC Opus
        +--> decoder scheduler --> native or isolated sidecar --> events
        +--> recording/playback service
        |
Versioned HTTP/WebSocket/WebRTC API
        |
Svelte UI --> PWA | Capacitor mobile | Tauri desktop
```

The initial process boundary is one Rust server plus isolated decoder
processes. Internal packages still communicate through explicit contracts so
hardware, DSP, media, and scheduling can be separated later without rewriting
the API.

## Engineering phases

### Phase 0: make the repository trustworthy

- Convert the Rust backend into a real workspace with one library and explicit
  `pulsescope-server` and optional Tauri binaries.
- Remove duplicated module/test compilation, fixture-backed success responses,
  dead API routes, and warning debt.
- Inventory the existing open pull requests and port useful changes in dependency
  order rather than merging overlapping branches wholesale.
- Repair the multi-stage Docker build so it builds the Svelte UI, Rust server,
  Soapy modules, health probe, and non-root runtime reproducibly.
- Add formatting, lint, unit, frontend, API contract, Docker build, software-IQ,
  and image smoke-test gates.
- Replace process-only health with advancing sample, FFT, audio, and event
  counters. A mock source must be explicitly labelled and can never satisfy a
  hardware readiness claim.

Exit gate: a clean checkout builds one image, starts with a synthetic IQ source,
serves the UI, streams continuous audio/waterfall, and passes all CI gates.

### Phase 1: rebuild the hot path

- Introduce a `RadioDevice` adapter contract with stable identity, RF ranges,
  sample rates, usable bandwidth, antennas, named gain stages, settings, stream
  MTU, counters, and hotplug state.
- Keep SoapySDR as the compatibility adapter. Add direct adapters when measured
  performance or lifecycle behavior requires them.
- Replace the mutex-backed fan-out with bounded SPSC readers over a timestamped
  sample block pool. Consumers have independent cursors and explicit drop
  counters; a slow decoder cannot stall audio or capture.
- Read blocks sized from the device stream MTU. Never hardcode 2 MSPS, 100 MHz,
  a model-name bandwidth, or a gain setting.
- Add sustained startup probes that choose the highest clean sample rate while
  retaining a safe fallback. Report total and usable bandwidth separately.
- Replace boxcar decimation with stateful polyphase FIR resampling. Use a PFB
  channelizer when receiver count makes independent DDCs more expensive.
- Make retunes and profile changes atomic and observable.

Exit gate: RSP1B streams for eight hours with no unreported gaps and recovers
from unplug/replug without restarting the full stack.

### Phase 2: production receiver media

- Implement AM, SAM, USB, LSB, CW, NFM, and WFM as stateful receiver pipelines
  with tested filters and level behavior.
- WFM processes sufficient-rate discriminator audio through a pilot PLL, stereo
  difference recovery, stereo blend, 15 kHz filtering, 50/75 microsecond
  de-emphasis, DC blocking, normalization, soft limiting, and RDS extraction.
- Packetize exactly 20 ms of 48 kHz PCM per Opus frame. Use full-band stereo
  audio mode for WFM and mono audio mode for communications receivers.
- Use WebRTC for audio and expose its loss, jitter, jitter-buffer, RTT, concealment,
  and playout-delay statistics in diagnostics.
- Keep recording taps before and after processing for repeatable audio-quality
  tests.
- Generate RF/audio fixtures and objective gates for frequency response,
  clipping, THD+N, stereo separation, discontinuities, and retune recovery.

Exit gate: clean WFM fixture THD+N below 1 percent, stereo separation above
30 dB, no clipping at nominal deviation, and two-hour LAN playback without a
client underrun.

### Phase 3: spectrum and user experience

- Stream quantized spectrum frames with protocol version, receiver ID, sequence,
  capture time, center, sample rate, usable span, bin count, floor, and scale.
- Bound queues at every boundary and discard obsolete spectrum frames rather
  than allowing latency to accumulate.
- Render and scroll the waterfall in a worker with one animation scheduler and
  visibility-aware rates.
- Rebuild the receiver screen around device, band/station, frequency, mode,
  volume, spectrum, decoder suggestions, and human-readable recovery state.
- Generate Expert controls from device capabilities. Unsupported controls never
  appear and unsupported values are rejected rather than silently substituted.
- Meet mobile layouts from 360 CSS pixels upward and desktop keyboard workflows
  without maintaining separate UIs.

Exit gate: 50 FPS desktop, 30 FPS mobile, no normal main-thread task over 100 ms,
and correct recovery UI during simulated USB, server, and network failures.

### Phase 4: decoder platform

- Define signed/versioned manifests for required input, bandwidth, tuning,
  executable or image checksum, resources, health, typed parameters, and event
  schema.
- Schedule decoders against existing IQ windows or allocate receivers based on
  bandwidth and tuner ownership.
- Feed digital voice from discriminator audio, not voice-filtered speaker audio.
- Keep mature tools isolated: `dsd-fme`/`dsd-neo`, `rtl_433`, `direwolf`,
  WSJT-X tools, `dumpvdl2`, `dump978`, `nrsc5`, `AIS-catcher`, `acarsdec`,
  `multimon-ng`, and SatDump.
- Retain native Rust decoders only when they have RF fixtures, conformance tests,
  and measurable performance advantages.
- Normalize all outputs into versioned events with frequency, protocol, quality,
  identifiers, location when applicable, encryption state, raw provenance, and
  timestamps. Encrypted traffic is labelled, never decrypted.

Priority: existing PulseScope ADS-B/AIS/APRS/POCSAG/RDS work, digital voice,
rtl_433, FT8/WSPR, ACARS/VDL2/HFDL, UAT, paging, trunk following, then satellite
and wideband services.

### Phase 5: packaging and clients

- Publish tested Compose profiles for core hardware families and decoder packs.
- Implement license-aware proprietary driver installation into a persistent,
  independently rollbackable driver volume.
- Add the PWA installation and background-audio gates before native shells.
- Build Capacitor mobile shells and Tauri desktop wrappers from the same UI
  release and generated API client.
- Add LAN discovery, QR pairing, TLS, scoped user tokens, server switching,
  backups, updates, and rollback.
- Maintain one semantic version across server, API schema, UI, mobile shells,
  desktop wrappers, and decoder catalog compatibility ranges.

## Initial cutover milestone

The first deployable slice is deliberately small:

1. Docker image and Compose start cleanly on the LAN host.
2. The RSP1B is discovered and capability controls are visible.
3. A stability probe selects 2, 5, or 10 MSPS based on measured loss.
4. FM broadcast produces continuous high-quality stereo audio and RDS.
5. NFM, AM, USB, and LSB are usable.
6. The waterfall meets desktop and mobile frame targets.
7. Unplug/replug recovery works without a manual container restart.
8. PWA installation and background audio work on the target phones.
9. The old archive remains available for rollback until a 72-hour soak passes.
10. Only then does PulseScope take port 8080.

## Definition of "works without hassle"

- `docker compose up -d` is the only shell command required after Docker is
  installed.
- The browser completes hardware and driver setup without arbitrary commands.
- Attach, detach, restart, upgrade, and browser reconnect are self-healing.
- Defaults are selected from measured capabilities and stability, not device-name
  guesses.
- Every failure names the affected component and gives one actionable remedy.
- Desktop users get the same experience from a signed installer without first
  installing Docker.
- Mobile users install a PWA or app, discover the server, pair, and listen; they
  do not configure SDR drivers or decoder executables.

## Non-goals for the first release

- Claiming that every future SDR is equally tested.
- Running Docker or proprietary desktop SDR drivers inside iOS.
- Shipping unverified decoder labels merely because an endpoint or UI panel
  exists.
- Reproducing every UltraSDR control before the core audio and spectrum gates
  pass.
