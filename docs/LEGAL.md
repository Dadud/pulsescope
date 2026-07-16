# Legal differentiation statement

PulseScope is a **clean-room reimplementation** of the *architecture* and
*user experience* common to the desktop SDR scanner category. It is not a
fork, decompilation, or derivative work of any proprietary scanner product.

## What is reused (and why it's legal)

| Element                              | Source                              | Legal basis |
|--------------------------------------|-------------------------------------|-------------|
| FFT, DSP, demodulation math          | Public-domain math                  | Ideas/math are not copyrightable |
| Scan-range frequencies (band edges)  | ITU / FCC / amateur allocations     | Public regulatory facts |
| Decoder binaries (rtl_433, etc.)     | Upstream open-source GPL projects   | GPL permits any use; we invoke as separate processes |
| SoapySDR device API                  | Boost Software License              | Permissive — compatible with MIT core |
| Protocol specs (P25 TIA-102, NXDN, ADS-B, ACARS, VDL2) | Public standards bodies | Specifications aren't code; implementations are original |

## What is NOT copied

- **No source code** from any proprietary scanner product is included,
  referenced, decompiled, or linked.
- **No wordmarks, trademarks, or trade dress** appear in PulseScope. The
  name "PulseScope", the pulse-wave logo, and the teal/slate color scheme
  are original.
- **No assets** (icons, sounds, UI textures, database dumps) are copied.
- **No frequency-lookup cloud API** is proxied. Lookups use RadioReference's
  SOAP API or a local database only.

## Architectural choices that mirror the category (and why that's fine)

The desktop SDR scanner UX pattern — a wideband spectrum display, a sidebar
of scan-range presets, multiple VFO tiles under a squelch engine, a decoded-
message log, and a trunking controller — is a **functional** design dictated
by the physics of radio scanning. Functional ideas, layouts dictated by
utility, and common vocabulary (VFO, squelch, waterfall) are not protectable
expression. PulseScope's implementation of each element is original code.

The local HTTP/WS API on `127.0.0.1:8765` follows the same observation: many
desktop SDR tools embed a local server for their webview to talk to. The
endpoint paths here were chosen to be familiar to users moving between
tools, but the handlers, types, and message formats are original Rust code.

## Clean-room methodology

1. **Behavioral spec** — extracted by observing the running application's
   config file, SQLite schema, and HTTP/WS endpoints from outside (no
   disassembly or source access).
2. **Public references** — DSP from textbook math; protocol formats from
   standards documents; decoder sidecar output formats from upstream docs.
3. **Original implementation** — all Rust and Svelte code in this repository
   was written from the behavioral spec and public references, not
   translated from any proprietary codebase.

If a contributor has prior access to any proprietary scanner's source, they
should not contribute to the corresponding module. The project maintainer
will tag clean-room-eligible modules in `docs/MODULE_OWNERSHIP.md`.
