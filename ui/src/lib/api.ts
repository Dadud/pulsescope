// api.ts — PulseScope local API client. The Rust backend binds a
// ws+http server to 127.0.0.1:8765; this module wraps the wire format.

// LAN/web builds use their own origin. Tauri v2 serves frontend assets from
// tauri.localhost (or an asset scheme), while PulseScope's API is a separate
// local server. Treating those origins as equivalent leaves the desktop UI
// blank/idle because every fetch targets the asset host instead of the API.
const desktopWebview = typeof window !== 'undefined' && (
  // Tauri v2 injects one of these regardless of whether WebView2 uses
  // tauri.localhost, asset:, or plain localhost as its visible origin.
  '__TAURI_INTERNALS__' in (window as Window & { __TAURI_INTERNALS__?: unknown; __TAURI__?: unknown }) || '__TAURI__' in (window as Window & { __TAURI_INTERNALS__?: unknown; __TAURI__?: unknown }) ||
  window.location.hostname === 'tauri.localhost' ||
  window.location.hostname.endsWith('.tauri.localhost') ||
  window.location.protocol === 'asset:' ||
  window.location.protocol === 'tauri:'
);
const BASE = typeof window === 'undefined' || desktopWebview
  ? 'http://127.0.0.1:8765'
  : `${window.location.protocol}//${window.location.hostname}:${window.location.port || (window.location.protocol === 'https:' ? 443 : 80)}`;
const WS_BASE = typeof window === 'undefined' || desktopWebview
  ? 'ws://127.0.0.1:8765'
  : `${window.location.protocol === 'https:' ? 'wss' : 'ws'}://${window.location.hostname}:${window.location.port || (window.location.protocol === 'https:' ? 443 : 80)}`;

/// Resolve the auth token from URL/localStorage and return as a Bearer header.
/// Caches in localStorage so subsequent requests stay authenticated.
function authHeader(): Record<string, string> {
  if (typeof window === 'undefined') return {};
  let token: string | null = null;
  try {
    const qs = new URLSearchParams(window.location.search);
    let t = qs.get('token');
    // Hash routing is used by the static UI; accept #/settings?token=...
    // as well as the normal ?token=... form so LAN links are hard to misuse.
    if (!t) {
      const hashQuery = window.location.hash.split('?')[1];
      if (hashQuery) t = new URLSearchParams(hashQuery).get('token');
    }
    if (t) { token = t; localStorage.setItem('pst', t); }
    else { token = localStorage.getItem('pst'); }
  } catch {}
  return token ? { 'authorization': `Bearer ${token}` } : {};
}

export interface VfoState {
  id: number;
  frequency_hz: number;
  mode: string;
  muted: boolean;
  volume: number;
  audio_agc: boolean;
  squelch_open: boolean;
  strength_db: number;
}

export interface DecodedMessage {
  id?: number;
  frequency_hz: number;
  protocol: string;
  message_type: string;
  address: string;
  function_code: string;
  content: string;
  raw: string;
  encryption: string;
  timestamp_ms: number;
}

export interface ScanRange {
  name: string;
  start_hz: number;
  end_hz: number;
  mode: string;
  channel_bw_hz: number;
  max_vfos: number;
  enabled: boolean;
  dwell_ms: number;
  squelch_db: number;
  auto_squelch_mode: string;
  hold_ms: number;
  sample_rate_hz: number;
}

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];
export interface JsonObject { [key: string]: JsonValue | undefined }

export interface SpectrumPayload { range: string; bins: number[] }
export interface SignalHitPayload {
  frequency_hz: number; strength_db: number; snr_db: number; bandwidth_hz: number;
  protocol: string; family: string; confidence: number; decoder: string;
}
export interface TrunkingUpdatePayload extends JsonObject {
  system?: string; active_talkgroup?: string; locked?: boolean;
}
export interface SpectrumOccupancyPayload extends JsonObject { frequency_bucket_hz: number }

export type ScannerEvent =
  | { kind: 'Spectrum'; data: SpectrumPayload }
  | { kind: 'SignalHit'; data: SignalHitPayload }
  | { kind: 'VfoStates'; data: VfoState[] }
  | { kind: 'DecodedMessage'; data: DecodedMessage }
  | { kind: 'TrunkingUpdate'; data: TrunkingUpdatePayload }
  | { kind: 'SpectrumOccupancy'; data: SpectrumOccupancyPayload };

export interface OperationResponse { ok: boolean; error?: string }
export interface SignalEvent extends JsonObject { frequency_hz: number; timestamp_ms: number }
export interface Job extends JsonObject { id: number }
export interface JobCreateRequest extends JsonObject { name: string; kind: string; payload: JsonObject; next_run_ms: number }
export interface ScanBankUpdate extends JsonObject { enabled?: boolean; dwell_ms?: number; hold_ms?: number; max_vfos?: number; squelch_db?: number }
export interface DeviceStatus extends JsonObject { label: string; connected: boolean; center_freq_hz?: number; sample_rate?: number; driver: string }
export interface DeviceInfo extends JsonObject { key: string; driver: string }
export interface DeviceList extends JsonObject { devices: DeviceInfo[] }
export interface DeviceCapabilities extends JsonObject { capabilities?: DeviceCapabilities }
export interface BankUpdateResponse extends OperationResponse { bank: ScanRange }
export interface IdentificationResponse extends JsonObject { available?: boolean; reason: string }


export async function getJson<T = unknown>(path: string): Promise<T> {
  const r = await fetch(`${BASE}${path}`, { headers: { ...authHeader() } });
  if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
  return r.json();
}

export async function postJson<T = unknown>(path: string, body?: unknown): Promise<T> {
  const r = await fetch(`${BASE}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', ...authHeader() },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
  return r.json();
}

export async function putJson<T = unknown>(path: string, body: JsonObject): Promise<T> {
  const r = await fetch(`${BASE}${path}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json', ...authHeader() },
    body: JSON.stringify(body),
  });
  if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
  return r.json();
}

export async function deleteJson<T = unknown>(path: string): Promise<T> {
  const r = await fetch(`${BASE}${path}`, { method: 'DELETE', headers: { ...authHeader() } });
  if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`);
  return r.json();
}

export function openEvents(cb: (ev: ScannerEvent) => void): WebSocket {
  // Honor auth if PULSESCOPE_AUTH_TOKEN is set or the URL has ?token=...
  const headers = authHeader();
  // Browser WebSocket constructor doesn't accept headers, so encode the
  // token as a query param. Server accepts both Bearer and ?token=.
  const base = typeof window !== 'undefined' ? `${WS_BASE}/events` : `${WS_BASE}/events`;
  const sep = base.includes('?') ? '&' : '?';
  const auth = headers['authorization'] && headers['authorization'].startsWith('Bearer ');
  const tokenParam = auth ? `${sep}token=${encodeURIComponent(headers['authorization'].slice(7))}` : '';
  const ws = new WebSocket(`${base}${tokenParam}`);
  ws.onmessage = (e) => {
    try { cb(JSON.parse(e.data)); }
    catch (err) { console.warn('bad ws frame', err); }
  };
  ws.onclose = () => console.log('event ws closed');
  ws.onerror = (e) => console.warn('event ws error', e);
  return ws;
}

// Convenience wrappers. Endpoint-specific payloads stay here so route components
// remain small and the backend contract has a single source of truth.
export const Api = {
  health: () => getJson('/health'),
  spectrum: () => getJson<{ bins: number[]; range?: string | null; running?: boolean }>('/spectrum'),
  banks: () => getJson<ScanRange[]>('/channels/banks'),
  createBank: (body: JsonObject) => postJson('/channels/banks/create', body),
  deleteBank: (name: string) => postJson('/channels/banks/delete', { name }),
  scanConfig: () => getJson('/channels/scan-config'),
  scanStart: (range_name: string) => postJson('/channels/scan/start', { range_name }),
  scanStop: () => postJson('/channels/scan/stop'),
  vfoStates: () => getJson<VfoState[]>('/vfo/states'),
  vfoMute: (id: number, on: boolean) => postJson(`/vfo/${id}/mute`, { id, on }),
  vfoVolume: (id: number, value: number) => postJson(`/vfo/${id}/volume`, { id, value }),
  vfoFrequency: (id: number, frequency_hz: number) => postJson(`/vfo/${id}/frequency`, { frequency_hz }),
  vfoMode: (id: number, mode: string) => postJson(`/vfo/${id}/mode`, { mode }),
  vfoAgc: (id: number, on: boolean) => postJson(`/vfo/${id}/audio_agc`, { id, on }),
  vfoIdentify: (id: number) => postJson<IdentificationResponse>(`/vfo/${id}/identify`, { id }),
  devices: () => getJson<DeviceList>('/devices'),
  deviceConnect: (key: string, label?: string) => postJson('/device/connect', { key, label }),
  deviceDisconnect: () => postJson('/device/disconnect'),
  deviceStatus: () => getJson<DeviceStatus>('/device/status'),
  deviceCapabilities: () => getJson<DeviceCapabilities>('/device/capabilities'),
  deviceControl: (control: string, value: string | number | boolean) => postJson<DeviceCapabilities>('/device/control', { control, value: String(value) }),
  decodedMessages: (limit = 100) => getJson<DecodedMessage[]>(`/decoded_messages?limit=${limit}`),
  signalEvents: (limit = 100) => getJson<SignalEvent[]>(`/signal_events?limit=${limit}`),
  channelBankScanConfig: () => getJson('/channels/bank-scan-config'),
  updateChannelBank: (name: string, body: ScanBankUpdate) => putJson<BankUpdateResponse>(`/channels/bank-scan-config`, { ...body, bank_name: name }),
  settings: () => getJson('/settings'),
  setSettings: (cfg: JsonObject) => putJson('/settings', cfg),
  trunkingStatus: () => getJson('/trunking/status'),
  trunkingStart: () => postJson('/trunking/start'),
  trunkingStop: () => postJson('/trunking/stop'),
  trunkingLock: (locked: boolean) => postJson('/trunking/lock', { locked }),
  trunkingCalls: () => getJson<JsonObject[]>('/trunking/calls'),
  trunkingDiscoveryStart: () => postJson('/trunking/discovery/start'),
  trunkingDiscoveryStop: () => postJson('/trunking/discovery/stop'),
  trunkingDiscoveryResults: () => getJson<JsonObject[]>('/trunking/discovery/results'),

  aeroStatus: () => getJson('/aero/status'),
  aeroMessages: () => getJson<JsonObject[]>('/aero/messages'),
  aeroEnable: (enabled: boolean) => postJson('/aero/enable', { enabled }),
  aeroClear: () => postJson('/aero/clear'),

  iridiumStatus: () => getJson('/iridium/status'),
  iridiumMessages: () => getJson<JsonObject[]>('/iridium/messages'),
  iridiumEnable: (enabled: boolean) => postJson('/iridium/enable', { enabled }),
  iridiumClear: () => postJson('/iridium/clear'),
  iridiumQuickStart: () => postJson('/iridium/quick-start'),

  gpsStatus: () => getJson('/gps/status'),
  glonassStatus: () => getJson('/glonass/status'),
  goesStatus: () => getJson('/goes_lrit/status'),
  satelliteEnable: (system: 'gps' | 'glonass' | 'goes', enabled: boolean) =>
    postJson(`/${system === 'goes' ? 'goes_lrit' : system}/enable`, { enabled }),
  satelliteClear: (system: 'gps' | 'glonass' | 'goes') =>
    postJson(`/${system === 'goes' ? 'goes_lrit' : system}/clear`),

  hdRadioStatus: () => getJson('/hd_radio/status'),
  hdRadioMessages: () => getJson<JsonObject[]>('/hd_radio/messages'),
  hdRadioCheck: () => postJson('/hd_radio/check'),
  hdRadioEnable: (enabled: boolean) => postJson('/hd_radio/enable', { enabled }),

  bleDevices: () => getJson<JsonObject[]>('/ble/devices'),
  bleStatus: () => getJson('/ble/status'),
  bleClear: () => postJson('/ble/clear'),
  loraMessages: () => getJson<JsonObject[]>('/lora/messages'),
  loraRegions: () => getJson<JsonObject[]>('/lora/regions'),

  signalFingerprints: () => getJson<JsonObject[]>('/signal_id/fingerprints'),
  signalSegmentBursts: () => postJson('/signal_id/segment_bursts'),
  signalPolyphaseExtract: () => postJson('/signal_id/polyphase_extract'),
  spectrumOccupancy: () => getJson('/spectrum_occupancy'),

  jobs: () => getJson<{ jobs: Job[] }>('/jobs'),
  createJob: (body: JobCreateRequest) => postJson('/jobs', body),
  deleteJob: (id:number) => deleteJson(`/jobs/${id}`),
  iqRecordingStatus: () => getJson('/iq_recording/status'),
  iqRecordingStart: () => postJson('/iq_recording/start'),
  iqRecordingStop: () => postJson('/iq_recording/stop'),
  cases: () => getJson<JsonObject[]>('/cases'),
  createCase: (body: JsonObject) => postJson('/cases', body),
  deleteCase: (id: number) => fetch(`${BASE}/cases/${id}`, { method: 'DELETE' }).then(r => r.json()),
  recordingAnnotations: () => getJson<JsonObject[]>('/recordings/annotations'),
  addRecordingAnnotation: (body: JsonObject) => postJson('/recordings/annotations', body),
  deleteRecordingAnnotation: (id: number) => fetch(`${BASE}/recordings/annotations/${id}`, { method: 'DELETE' }).then(r => r.json()),
  transcriptionStatus: () => getJson('/transcription/status'),
  transcripts: () => getJson<JsonObject[]>('/transcription/transcripts'),
  transcriptionStart: () => postJson('/transcription/start'),
  transcriptionStop: () => postJson('/transcription/stop'),

  featurePacks: () => getJson('/feature-packs'),
  featurePackEnable: (id: string, enabled: boolean) => postJson(`/feature-packs/${encodeURIComponent(id)}/enable`, { enabled }),
  sidecarsStatus: () => getJson<JsonObject[]>('/sidecars/status'),
  sidecarStderr: (name: string) => getJson<string[]>(`/sidecars/${encodeURIComponent(name)}/stderr`),
  blacklist: () => getJson('/blacklist'),
  blacklistAdd: (frequency_hz: number, reason = '') => postJson('/blacklist/add', { frequency_hz, reason }),
  blacklistRemove: (frequency_hz: number) => postJson('/blacklist/remove', { frequency_hz }),
  blacklistClear: () => postJson('/blacklist/clear'),

  debugStats: () => getJson('/debug/stats'),
  debugLogTail: () => getJson('/debug/log/tail'),
  debugNoiseFloor: () => getJson('/debug/noise_floor'),
  debugClassifications: () => getJson('/debug/classifications'),
  debugDsdStderr: () => getJson('/debug/dsd_stderr'),
  debugVdl2Stderr: () => getJson('/debug/vdl2_stderr'),
  debugRtl433Stderr: () => getJson('/debug/rtl433_stderr'),

  aircraftLookup: (query: string) => getJson(`/aircraft/lookup?q=${encodeURIComponent(query)}`),
};
