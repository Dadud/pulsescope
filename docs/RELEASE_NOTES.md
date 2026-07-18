# PulseScope pre-1.0 release notes

## Complete

- Mock-source core tests, using the command recorded in the acceptance matrix.

## Experimental

- Windows MSI/NSIS, Ubuntu headless/desktop packages, and Debian containers are CI
  targets. Their complete clean-install, upgrade, rollback, startup, and uninstall
  evidence has not yet been recorded.
- Auth/TLS, IQ playback/recording, streaming, native decoders, and database
  migration await required acceptance fixtures.

## Dependency-gated

- Physical SDR operation requires compatible hardware and a SoapySDR module.
- rtl_433, dsd-fme, and other external decoders are installed separately.

## Unavailable for 1.0

- macOS, Windows ARM64, 32-bit targets, and distributions outside the support
  matrix are unsupported.

No 1.0 tag is approved. Structured acceptance evidence, not historical prose or
local-machine tests, is the release-readiness record.
