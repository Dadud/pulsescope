# Feature status

A native decoder is marked **complete** only when a licensed/provenanced corpus covers valid, corrupted, weak, truncated, and back-to-back frames and an end-to-end test demonstrates IQ capture → scanner routing → physical/link decoding → versioned persistence with raw frame retention → event emission → HTTP retrieval → UI-compatible JSON. Unit-tested parsers or sidecar endpoints alone are **in progress**.

| Decoder | Native interface | Physical layer | Routing | Stable persistence | Corpus + E2E | Status |
|---|---:|---:|---:|---:|---:|---|
| ADS-B 1090ES | yes | yes | yes | yes | partial | In progress |
| AIS | yes | in progress | yes | yes | partial | In progress |
| APRS | yes | in progress | yes | yes | partial | In progress |
| POCSAG | yes | yes | yes | yes | partial | In progress |
| UAT / ACARS / VDL2 | yes | in progress | yes | yes | partial | In progress |
| RDS / DCS metadata | n/a | in progress | classifier | n/a | partial | In progress |

The `/decoders` API and Decoded Messages page expose availability and live counters. A decoder moves to **complete** only when every column through Corpus + E2E is `yes` in CI.
