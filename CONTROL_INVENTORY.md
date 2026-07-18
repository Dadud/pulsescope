# UI control and route inventory

All route controls below are backed by the named API operation. A disabled control must include an inline availability reason and `aria-describedby`; placeholder actions are not permitted.

| Route | Controls / operation |
|---|---|
| `/` | range/quick scan (`scanStart`, `scanStop`), spectrum tune and VFO frequency/mode/mute/AGC/identify, message filter/export/clear, waterfall display controls |
| `/settings` | connect receiver, refresh capabilities, save/delete banks, save settings |
| `/trunking` | start/stop, lock, discovery, refresh |
| `/aero`, `/iridium`, `/satellites`, `/hd-radio` | enable/disable, clear/check/quick-start, refresh |
| `/ble` | filter, refresh, clear |
| `/lora`, `/occupancy`, `/debug` | refresh |
| `/signal-id` | segment bursts, polyphase extraction, refresh |
| `/recording` | start/stop IQ and transcription, refresh |
| `/jobs` | schedule and delete jobs |
| `/cases` | create and delete case |
| `/aircraft` | keyboard-accessible lookup |
| `/lookups` | edit, save, reload provider configuration |
| `/feature-packs` | enable/disable and refresh |
| `/blacklist` | add, remove, clear, refresh |
| `/deps` | refresh and fetch install guide |
| `/messages` | search/filter and refresh |

The former Hold, per-VFO recording, and VFO zoom placeholders were removed from the scanner. Recording remains available from `/recording`; zoom is available through browser/canvas accessibility controls when implemented by the backend.
