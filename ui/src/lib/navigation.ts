/** Flat, grouped navigation — every destination is one click away. */
export type NavItem = {
  label: string;
  href: string;
  description?: string;
};

export type NavSection = {
  id: string;
  label: string;
  items: NavItem[];
};

export const navSections: NavSection[] = [
  {
    id: 'scan',
    label: 'Scan',
    items: [
      { label: 'Receiver', href: '#/', description: 'Spectrum, waterfall, VFOs, tuning' },
      { label: 'Monitor', href: '#/monitor', description: 'System health and decoder jobs' },
      { label: 'Occupancy', href: '#/occupancy', description: 'Spectrum utilization meters' },
      { label: 'Signal ID', href: '#/signal-id', description: 'Fingerprints and burst segmentation' },
      { label: 'Blacklist', href: '#/blacklist', description: 'Frequency exclusion list' },
    ],
  },
  {
    id: 'decode',
    label: 'Decode',
    items: [
      { label: 'Messages', href: '#/messages', description: 'Full decoded message log' },
      { label: 'Trunking', href: '#/trunking', description: 'P25, NXDN, EDACS, DMR' },
      { label: 'Aero', href: '#/aero', description: 'Inmarsat AERO' },
      { label: 'Iridium', href: '#/iridium', description: 'Iridium burst decoder' },
      { label: 'Satellites', href: '#/satellites', description: 'GPS, GLONASS, GOES LRIT' },
      { label: 'HD Radio', href: '#/hd-radio', description: 'NRSC5 FM HD Radio' },
      { label: 'BLE', href: '#/ble', description: 'Bluetooth Low Energy' },
      { label: 'LoRa', href: '#/lora', description: 'LoRa regional plans' },
    ],
  },
  {
    id: 'record',
    label: 'Record & cases',
    items: [
      { label: 'Recording', href: '#/recording', description: 'IQ capture, transcription, annotations' },
      { label: 'Cases', href: '#/cases', description: 'Investigation case management' },
      { label: 'Jobs', href: '#/jobs', description: 'Scheduled scan and recording jobs' },
    ],
  },
  {
    id: 'tools',
    label: 'Tools',
    items: [
      { label: 'Aircraft', href: '#/aircraft', description: 'ICAO, registration, callsign lookup' },
      { label: 'Lookups', href: '#/lookups', description: 'RadioReference and FCC lookup' },
      { label: 'Feature packs', href: '#/feature-packs', description: 'Enable optional decoder packs' },
      { label: 'Dependencies', href: '#/deps', description: 'Decoder runtime and install status' },
    ],
  },
  {
    id: 'system',
    label: 'System',
    items: [
      { label: 'Profiles', href: '#/profiles', description: 'Hardware profiles and bookmarks' },
      { label: 'Settings', href: '#/settings', description: 'Device, receiver, scanner, audio' },
      { label: 'Debug', href: '#/debug', description: 'Stats, logs, classifier state' },
    ],
  },
];

/** All navigable destinations for the command palette. */
export const allNavItems: NavItem[] = navSections.flatMap((s) => s.items);

/** Normalize hash route for active-state comparison. */
export function normalizeRoute(hashOrPath: string): string {
  const raw = hashOrPath.startsWith('#') ? hashOrPath.slice(1) : hashOrPath;
  if (!raw || raw === '/') return '/';
  return raw.startsWith('/') ? raw : `/${raw}`;
}

export function isRouteActive(current: string, href: string): boolean {
  return normalizeRoute(current) === normalizeRoute(href.replace('#', ''));
}
