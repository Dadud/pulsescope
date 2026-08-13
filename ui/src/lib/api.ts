// api.ts — PulseScope local API client. The Rust backend binds a
// ws+http server to 127.0.0.1:8765; this module wraps the wire format.

// LAN/web builds use their own origin. Tauri v2 serves frontend assets from
// tauri.localhost (or an asset scheme), while PulseScope's API is a separate
// local server. Treating those origins as equivalent leaves the desktop UI
// blank/idle because every fetch targets the asset host instead of the API.
const desktopWebview = typeof window !== 'undefined' && (
  // Tauri v2 injects one of these regardless of whether WebView2 uses
  // tauri.localhost, asset:, or plain localhost as its visible origin.
  '__TAURI_INTERNALS__' in (window as any) || '__TAURI__' in (window as any) ||
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
  audio_level_db?: number;
  locked?: boolean;
  last_hit_ms?: number;
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

export interface ScannerEvent {
  kind: 'Spectrum' | 'SignalHit' | 'VfoStates' | 'DecodedMessage' | 'TrunkingUpdate' | 'SpectrumOccupancy';
  data: any;
}

export interface PcmAudioFrame {
  sequence: number;
  sampleRate: number;
  channels: number;
  capturedMs: number;
  samples: Float32Array;
}

export interface SpectrumStreamFrame {
  sequence: number;
  capturedMs: number;
  centerFreqHz: number;
  sampleRateHz: number;
  usableSpanHz: number;
  bins: number[];
  receiverId: number;
  sessionRevision: number;
}

const LIVE_REQUEST_TIMEOUT_MS = 5_000;

export class ApiError extends Error {
  constructor(
    message: string,
    readonly path: string,
    readonly status: number,
    readonly retryable: boolean,
    readonly requestId?: string,
  ) { super(message); this.name = 'ApiError'; }
}

async function fetchBounded(input: string, init: RequestInit = {}): Promise<Response> {
  const controller = new AbortController();
  const timeout = globalThis.setTimeout(() => controller.abort(), LIVE_REQUEST_TIMEOUT_MS);
  try {
    return await fetch(input, { cache: 'no-store', ...init, signal: controller.signal });
  } finally {
    globalThis.clearTimeout(timeout);
  }
}

async function throwHttpError(path: string, response: Response): Promise<never> {
  let message = `HTTP ${response.status}`;
  try {
    const payload = await response.clone().json();
    message = payload?.error || payload?.message || payload?.detail || message;
  } catch { /* non-JSON error bodies are still represented by status */ }
  const requestId = response.headers.get('x-request-id') ?? undefined;
  throw new ApiError(`${path}: ${message}`, path, response.status, response.status >= 500 || response.status === 408 || response.status === 429, requestId);
}

export async function getJson<T = any>(path: string): Promise<T> {
  const r = await fetchBounded(`${BASE}${path}`, { headers: { ...authHeader() } });
  if (!r.ok) await throwHttpError(path, r);
  return r.json();
}

export async function postJson<T = any>(path: string, body?: any): Promise<T> {
  const r = await fetchBounded(`${BASE}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', ...authHeader() },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!r.ok) await throwHttpError(path, r);
  return r.json();
}

export async function putJson<T = any>(path: string, body: any): Promise<T> {
  const r = await fetchBounded(`${BASE}${path}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json', ...authHeader() },
    body: JSON.stringify(body),
  });
  if (!r.ok) await throwHttpError(path, r);
  return r.json();
}

export async function deleteJson<T = any>(path: string): Promise<T> {
  const r = await fetchBounded(`${BASE}${path}`, { method: 'DELETE', headers: { ...authHeader() } });
  if (!r.ok) await throwHttpError(path, r);
  return r.json();
}

function websocketUrl(path: string): string {
  // Honor auth if PULSESCOPE_AUTH_TOKEN is set or the URL has ?token=...
  const headers = authHeader();
  // Browser WebSocket constructor doesn't accept headers, so encode the
  // token as a query param. Server accepts both Bearer and ?token=.
  const base = `${WS_BASE}${path}`;
  const sep = base.includes('?') ? '&' : '?';
  const auth = headers['authorization'] && headers['authorization'].startsWith('Bearer ');
  const tokenParam = auth ? `${sep}token=${encodeURIComponent(headers['authorization'].slice(7))}` : '';
  return `${base}${tokenParam}`;
}

export function openEvents(
  cb: (ev: ScannerEvent) => void,
  onState?: (state: 'connecting' | 'open' | 'closed' | 'error') => void,
): WebSocket {
  onState?.('connecting');
  const ws = new WebSocket(websocketUrl('/events'));
  ws.onopen = () => onState?.('open');
  ws.onmessage = (e) => {
    try { cb(JSON.parse(e.data)); }
    catch (err) { console.warn('bad ws frame', err); }
  };
  ws.onclose = () => onState?.('closed');
  ws.onerror = () => onState?.('error');
  return ws;
}

export function openSpectrum(
  onFrame: (frame: SpectrumStreamFrame) => void,
  onState?: (state: 'connecting' | 'open' | 'closed' | 'error') => void,
): WebSocket {
  onState?.('connecting');
  const ws = new WebSocket(websocketUrl('/api/v2/spectrum/stream'));
  ws.binaryType = 'arraybuffer';
  ws.onopen = () => onState?.('open');
  ws.onclose = () => onState?.('closed');
  ws.onerror = () => onState?.('error');
  ws.onmessage = (event) => {
    if (!(event.data instanceof ArrayBuffer) || event.data.byteLength < 64) return;
    const bytes = new Uint8Array(event.data);
    if (String.fromCharCode(...bytes.subarray(0, 4)) !== 'PSF3') return;
    const view = new DataView(event.data);
    if (view.getUint16(4, true) !== 3) return;
    const count = view.getUint32(40, true);
    if (64 + count !== event.data.byteLength) return;
    const floor = view.getFloat32(44, true);
    const scale = view.getFloat32(48, true);
    const bins = Array.from(bytes.subarray(64), (value) => floor + value * scale);
    onFrame({
      sequence: Number(view.getBigUint64(8, true)),
      capturedMs: Number(view.getBigInt64(16, true)),
      centerFreqHz: Number(view.getBigUint64(24, true)),
      sampleRateHz: view.getUint32(32, true),
      usableSpanHz: view.getUint32(36, true),
      receiverId: view.getUint32(52, true),
      sessionRevision: Number(view.getBigUint64(56, true)),
      bins,
    });
  };
  return ws;
}

export function openAudio(
  onFrame: (frame: PcmAudioFrame) => void,
  onState?: (state: 'connecting' | 'open' | 'closed' | 'error') => void,
): WebSocket {
  onState?.('connecting');
  const ws = new WebSocket(websocketUrl('/audio/stream'));
  ws.binaryType = 'arraybuffer';
  ws.onopen = () => onState?.('open');
  ws.onclose = () => onState?.('closed');
  ws.onerror = () => onState?.('error');
  ws.onmessage = (event) => {
    if (!(event.data instanceof ArrayBuffer) || event.data.byteLength < 32) return;
    const bytes = new Uint8Array(event.data);
    if (String.fromCharCode(...bytes.subarray(0, 4)) !== 'PSA2') return;
    const view = new DataView(event.data);
    if (view.getUint16(4, true) !== 2) return;
    const channels = view.getUint16(6, true);
    const sampleRate = view.getUint32(8, true);
    const sequence = Number(view.getBigUint64(12, true));
    const capturedMs = Number(view.getBigInt64(20, true));
    const sampleCount = view.getUint32(28, true);
    if (channels < 1 || sampleRate < 8_000 || 32 + sampleCount * 4 !== event.data.byteLength) return;
    const samples = new Float32Array(sampleCount);
    for (let i = 0; i < sampleCount; i += 1) samples[i] = view.getFloat32(32 + i * 4, true);
    onFrame({ sequence, sampleRate, channels, capturedMs, samples });
  };
  return ws;
}

// Convenience wrappers. Endpoint-specific payloads stay here so route components
// remain small and the backend contract has a single source of truth.
export const Api = {
  health: () => getJson('/health'),
  spectrum: () => getJson<{ bins: number[]; range?: string | null; running?: boolean; frame_sequence?: number; frame_timestamp_ms?: number }>('/spectrum'),
  banks: () => getJson<ScanRange[]>('/channels/banks'),
  createBank: (body: any) => postJson('/channels/banks/create', body),
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
  vfoIdentify: (id: number) => postJson(`/vfo/${id}/identify`, { id }),
  devices: () => getJson('/devices'),
  deviceConnect: (key: string, label?: string) => postJson('/device/connect', { key, label }),
  deviceDisconnect: () => postJson('/device/disconnect'),
  deviceStatus: () => getJson('/device/status'),
  deviceFrequency: (frequency_hz: number) => postJson('/device/frequency', { frequency_hz }),
  deviceSampleRate: (sample_rate: number) => postJson('/device/sample_rate', { sample_rate }),
  deviceCapabilities: () => getJson('/device/capabilities'),
  deviceControl: (control: string, value: string | number | boolean) => postJson('/device/control', { control, value: String(value) }),
  decodedMessages: (limit = 100) => getJson<DecodedMessage[]>(`/decoded_messages?limit=${limit}`),
  signalEvents: (limit = 100) => getJson<any[]>(`/signal_events?limit=${limit}`),
  channelBankScanConfig: () => getJson('/channels/bank-scan-config'),
  updateChannelBank: (name: string, body: any) => putJson(`/channels/bank-scan-config`, { ...body, bank_name: name }),
  settings: () => getJson('/settings'),
  setSettings: (cfg: any) => putJson('/settings', cfg),
  trunkingStatus: () => getJson('/trunking/status'),
  trunkingStart: () => postJson('/trunking/start'),
  trunkingStop: () => postJson('/trunking/stop'),
  trunkingLock: (locked: boolean) => postJson('/trunking/lock', { locked }),
  trunkingCalls: () => getJson<any[]>('/trunking/calls'),
  trunkingDiscoveryStart: () => postJson('/trunking/discovery/start'),
  trunkingDiscoveryStop: () => postJson('/trunking/discovery/stop'),
  trunkingDiscoveryResults: () => getJson<any[]>('/trunking/discovery/results'),

  aeroStatus: () => getJson('/aero/status'),
  aeroMessages: () => getJson<any[]>('/aero/messages'),
  aeroEnable: (enabled: boolean) => postJson('/aero/enable', { enabled }),
  aeroClear: () => postJson('/aero/clear'),

  iridiumStatus: () => getJson('/iridium/status'),
  iridiumMessages: () => getJson<any[]>('/iridium/messages'),
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
  hdRadioMessages: () => getJson<any[]>('/hd_radio/messages'),
  hdRadioCheck: () => postJson('/hd_radio/check'),
  hdRadioEnable: (enabled: boolean) => postJson('/hd_radio/enable', { enabled }),

  bleDevices: () => getJson<any[]>('/ble/devices'),
  bleStatus: () => getJson('/ble/status'),
  bleClear: () => postJson('/ble/clear'),
  loraMessages: () => getJson<any[]>('/lora/messages'),
  loraRegions: () => getJson<any[]>('/lora/regions'),

  signalFingerprints: () => getJson<any[]>('/signal_id/fingerprints'),
  signalSegmentBursts: () => postJson('/signal_id/segment_bursts'),
  signalPolyphaseExtract: () => postJson('/signal_id/polyphase_extract'),
  spectrumOccupancy: () => getJson('/spectrum_occupancy'),

  jobs: () => getJson<{jobs:any[]}>('/jobs'),
  createJob: (body:any) => postJson('/jobs', body),
  deleteJob: (id:number) => deleteJson(`/jobs/${id}`),
  iqRecordingStatus: () => getJson('/iq_recording/status'),
  iqRecordingStart: () => postJson('/iq_recording/start'),
  iqRecordingStop: () => postJson('/iq_recording/stop'),
  cases: () => getJson<any[]>('/cases'),
  createCase: (body: any) => postJson('/cases', body),
  deleteCase: (id: number) => fetch(`${BASE}/cases/${id}`, { method: 'DELETE' }).then(r => r.json()),
  recordingAnnotations: () => getJson<any[]>('/recordings/annotations'),
  addRecordingAnnotation: (body: any) => postJson('/recordings/annotations', body),
  deleteRecordingAnnotation: (id: number) => fetch(`${BASE}/recordings/annotations/${id}`, { method: 'DELETE' }).then(r => r.json()),
  transcriptionStatus: () => getJson('/transcription/status'),
  transcripts: () => getJson<any[]>('/transcription/transcripts'),
  transcriptionStart: () => postJson('/transcription/start'),
  transcriptionStop: () => postJson('/transcription/stop'),

  featurePacks: () => getJson('/feature-packs'),
  decoderCatalogV2: () => getJson<{ contract_version: number; decoders: any[] }>('/api/v2/decoders/catalog'),
  featureStatusV2: () => getJson('/api/v2/features'),
  featurePackEnable: (id: string, enabled: boolean) => postJson(`/feature-packs/${encodeURIComponent(id)}/enable`, { enabled }),
  sidecarsStatus: () => getJson<any[]>('/sidecars/status'),
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
