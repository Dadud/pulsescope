import type { DecodedMessage } from '$lib/api';

export interface DecoderAlertPrefs {
  enabled: boolean;
  protocols: string[];
}

const STORAGE_KEY = 'pulsescope.decoder-alerts';

export const DECODER_ALERT_PROTOCOL_OPTIONS = [
  { id: 'adsb', label: 'ADS-B / aircraft' },
  { id: 'ais', label: 'AIS / marine' },
  { id: 'aprs', label: 'APRS' },
  { id: 'pocsag', label: 'POCSAG / paging' },
  { id: 'meshtastic', label: 'Meshtastic' },
  { id: 'meshcore', label: 'MeshCore' },
  { id: 'p25-tsbk', label: 'P25 trunk grants' },
  { id: 'rtl433', label: 'rtl_433 sensors' },
  { id: 'acars', label: 'ACARS' },
] as const;

export function defaultDecoderAlertPrefs(): DecoderAlertPrefs {
  return {
    enabled: false,
    protocols: ['adsb', 'aprs', 'pocsag', 'p25-tsbk'],
  };
}

export function loadDecoderAlertPrefs(): DecoderAlertPrefs {
  if (typeof window === 'undefined') return defaultDecoderAlertPrefs();
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return defaultDecoderAlertPrefs();
    const parsed = JSON.parse(raw) as Partial<DecoderAlertPrefs>;
    return {
      enabled: Boolean(parsed.enabled),
      protocols: Array.isArray(parsed.protocols)
        ? parsed.protocols.filter((item): item is string => typeof item === 'string')
        : defaultDecoderAlertPrefs().protocols,
    };
  } catch {
    return defaultDecoderAlertPrefs();
  }
}

export function saveDecoderAlertPrefs(prefs: DecoderAlertPrefs) {
  if (typeof window === 'undefined') return;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs));
}

export async function requestDecoderAlertPermission(): Promise<
  NotificationPermission | 'unsupported'
> {
  if (typeof Notification === 'undefined') return 'unsupported';
  if (Notification.permission === 'granted') return 'granted';
  if (Notification.permission === 'denied') return 'denied';
  return Notification.requestPermission();
}

function protocolMatches(messageProtocol: string, selected: string[]) {
  const protocol = messageProtocol.toLowerCase();
  return selected.some((entry) => protocol.includes(entry.toLowerCase()));
}

function formatFrequency(hz: number) {
  if (hz >= 1e6) return `${(hz / 1e6).toFixed(3)} MHz`;
  if (hz >= 1e3) return `${(hz / 1e3).toFixed(1)} kHz`;
  return `${hz} Hz`;
}

const recentKeys = new Set<string>();

export function maybeNotifyDecodedMessage(
  message: DecodedMessage,
  prefs: DecoderAlertPrefs = loadDecoderAlertPrefs(),
) {
  if (!prefs.enabled || typeof Notification === 'undefined') return;
  if (Notification.permission !== 'granted') return;
  if (!protocolMatches(message.protocol ?? '', prefs.protocols)) return;

  const key = `${message.protocol}:${message.address}:${message.content}:${message.timestamp_ms ?? 0}`;
  if (recentKeys.has(key)) return;
  recentKeys.add(key);
  if (recentKeys.size > 200) recentKeys.clear();

  const title = `${message.protocol.toUpperCase()} · ${formatFrequency(Number(message.frequency_hz ?? 0))}`;
  const body = message.content || message.message_type || message.address || 'Decoder event';
  try {
    const notification = new Notification(title, {
      body,
      tag: key,
      silent: false,
    });
    notification.onclick = () => {
      window.focus();
      window.location.hash = message.protocol.toLowerCase().includes('p25')
        ? '#/trunking'
        : '#/messages';
      notification.close();
    };
  } catch (error) {
    console.warn('decoder alert notification failed', error);
  }
}
