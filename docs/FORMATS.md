# PulseScope capture and transport formats

All multi-byte integers and IEEE-754 values described here are little-endian. Writers must report short writes and close a recording cleanly on shutdown. Readers must reject incomplete headers and ignore a final incomplete sample/frame rather than reading beyond the file.

## CF32 (`.cf32`)

Headerless complex baseband IQ. Each 8-byte sample is `I:f32, Q:f32`, normalized nominally to `[-1, 1]`. Frequency, sample rate, start time, and hardware configuration are external metadata; consequently CF32 should only be exchanged with that metadata. A file whose length is not divisible by 8 is truncated.

## WAV (`.wav`)

Per-VFO demodulated audio uses RIFF/WAVE, mono signed PCM (`WAVE_FORMAT_PCM`), 16 bits per sample, at the rate declared in `fmt `. PulseScope writes a `psmd` chunk containing UTF-8 JSON before `data`. It includes `frequency_hz`, `mode`, `started_ms`, `signal_id`, `case_id`, and `case_name`; the authoritative sample rate is the `fmt ` rate. Unknown chunks must be skipped with RIFF word alignment. A missing/short `data` chunk is a truncated recording.

Live browser audio is the same PCM WAV representation with open-ended RIFF/data sizes (`0xffffffff`). It is an authenticated HTTP response at `/audio/vfo/{id}/stream.wav`. Each subscriber has a bounded 32-chunk queue; lagging clients drop old audio, and disconnecting a browser drops its receiver without affecting DSP or recording.

## PSAU

PSAU is the low-latency UDP audio datagram format. The 16-byte header is: magic `PSAU` (4), version `u16` (currently 1), sample rate `u32`, sample count `u16`, flags `u16`, reserved `u16`; this is followed by `sample_count` mono `f32` PCM samples. Datagram boundaries are frame boundaries. Receivers must reject bad magic/version, length mismatches, and unreasonable sample counts. UDP loss is represented as missing audio and must never block capture.

## PSIQ

PSIQ is the framed IQ network/capture format. Its 32-byte header is magic `PSIQ` (4), version `u16`, flags `u16`, center frequency `u64`, sample rate `u32`, sample count `u32`, sequence `u64`; payload samples are interleaved `I:f32, Q:f32`. Sequence gaps indicate loss. Readers must validate the complete header, checked-multiply sample count by eight, impose an implementation limit before allocation, and reject a short payload. Version 1 uses flags 0.

## Safety and recovery

Library APIs accept a single file name, never an absolute or relative path. The server canonicalizes existing files beneath its recordings root, rejects separators and `.`/`..`, refuses overwrite on rename, and serves downloads as attachments. WAV headers are finalized on stop and application shutdown. If disk or permission errors occur, the writer records the error and stops accepting samples; partially written files remain inspectable as truncated evidence rather than being silently deleted.
