# Decode-sidecar output parsers (next iteration)

Parser code is not acceptance evidence. Native decoders remain **experimental**
until every advertised decoder has a deterministic fixture in the acceptance
matrix. Process parsers are **dependency-gated**: executables are not bundled and
a missing binary must report unavailable rather than imply completeness.

Each sidecar decoder emits a distinctive line format on stdout. The Rust
`sidecar::parse_line` dispatcher needs one regex per protocol.

## rtl_433 (JSON mode, `-M json`)

```
{"time":"2026-07-14T21:32:39Z","model":"Acurite-Tower","id":12345,"channel":"A","battery_ok":1,"temperature_C":23.4,"humidity":56,"mic":"CRC"}
```
Protocol: `rtl_433`. Address: `id`. Content: `model + temperature_C + humidity`.

## dumpvdl2 (`--json` or default text)

```
[2026-07-14 21:32:39 UTC] [136.975 MHz] Msg #1: AC: ABC123, Label: H1, M: S, Text: ...
```
Protocol: `vdl2`. Address: `AC:` field. Content: `Text:` field.

## multimon-ng (`-a POCSAG512 -t raw`)

```
POCSAG512: Address:  123456  Function: 0  Alpha:   HELLO
```
Protocol: `pocsag`. Address: numeric. Content: `Alpha:` field.

## direwolf (AGWPE / KISS)

Direwolf emits APRS frames as TNC2 monitor format:
```
N0CALL>APRS,TCPIP*,qAC,T2:=4400.00N/09000.00W-Test beacon
```
Protocol: `aprs`. Address: sender callsign. Content: info field after `:`.

## acarsdec

```
[#1 (F:131.550) 14/07/2026 21:32:39 ABC123 L1 S O 1 Label: H1 M: ACARS
   Message: REG.ABC123.FLIGHT.NO1234 ...
```
Protocol: `acars`. Address: aircraft reg. Content: `Message:` block.

## rs41mod

```
2026-07-14T21:32:39Z [402.500 MHz] RS41 frame: serial=N1234567 lat=... lon=... alt=...
```
Protocol: `rs41`. Address: serial. Content: telemetry line.

## dsd-neo (P25 / DMR)

dsd-neo writes audio + metadata to stdout; trunking TGID / UID on lines like:
```
Sync: P25p1  TG: 1234  UID: 9999  Encrypted: NO
```
Protocol: `p25` / `dmr`. Address: TG. Content: TGID/UID + encryption status.
