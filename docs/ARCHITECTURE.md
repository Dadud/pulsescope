# PulseScope architecture

## Runtime ownership

The Rust server is authoritative. Browsers submit revisioned intent and render sequenced events; they do not own hardware or decoder processes.

```text
RadioDevice adapters
  -> timestamped bounded IQ block bus
     -> FFT service -> viewport spectrum stream -> worker renderer
     -> receiver allocator -> demodulators -> media service -> listener audio
     -> decoder scheduler -> isolated decoder -> normalized events
     -> recording/playback service
  -> HardwareWindow control and health

SQLite <-> profiles, bookmarks, jobs, events, recordings, calibration
HTTP/WS/WebRTC <-> Svelte Receiver and Monitor workspaces
```

## Hardware and IQ

`RadioDevice` publishes stable identity, RF ranges, antennas, formats, rates, bandwidths, stream MTU, named gains, settings, and counters. `HardwareWindow` is the single actual capture state for a tuner. Capture reads MTU-sized blocks into bounded independent consumers; a decoder or browser cannot stall capture, FFT, or audio.

Frequency, profile, rate, and bandwidth changes are atomic. Desired state is never reported as actual until driver readback succeeds. Hotplug moves through detected, probing, configuring, streaming, degraded, recovering, and ready.

## Receiver and media

A `ListenerSession` owns independent VFOs, viewport, mode, filter, squelch, audio selection, and attached decoders. Sessions share IQ already present in the hardware window. Profile changes are shared operations and use a revisioned countdown event.

Receiver pipelines consume timestamped IQ blocks and emit audio/discriminator/IQ taps plus level and quality metadata. WebRTC Opus is the target browser transport; timestamped 20 ms PCM WebSocket frames remain the compatibility path until parity gates pass.

## Spectrum and waterfall

The spectrum service produces sequence-numbered, timestamped frames with hardware and visible spans. Each browser subscribes with viewport and pixel requirements. Queues are latest-frame-only. The worker renderer owns waterfall history, WebGL resources, calibration, and device-pixel-ratio resizing; UI state owns viewport history and interaction intent.

## Decoders and events

Decoder manifests declare accepted input, tuning requirements, executable/image checksum, parameters, resources, health, restart policy, and normalized outputs. Scheduler jobs either share an existing IQ window or request tuner ownership. Arbitrary client command lines and decryption are prohibited.

Normalized events include protocol, frequency, capture time, identifiers, position, quality, encryption state, raw provenance, decoder version, and recording reference. Maps, activity views, notifications, and exports consume the same event contract.

## Persistence and compatibility

SQLite migrations are append-only. Public v2 commands are idempotent and revisioned; events are sequenced. Compatibility routes remain until generated clients and conformance tests no longer depend on them. The acceptance matrix, not endpoint presence, determines UI visibility and release claims.

## Security and operations

Default deployment is an unprivileged trusted-LAN service with narrowly scoped USB access. Remote exposure requires TLS and scoped authentication. Driver layers, data, recordings, and calibration are persistent and independently backed up. Production changes use preflight, an isolated canary, physical handoff, acceptance gates, and a preserved rollback image.
