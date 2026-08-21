/** Sidebar navigation: every category is collapsed until clicked (current route opens its group). */
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
    id: 'workspaces',
    label: 'Workspaces',
    items: [
      { label: 'Receiver', href: '#/', description: 'Spectrum, waterfall, VFOs, tuning' },
      { label: 'Monitor', href: '#/monitor', description: 'System health and decoder jobs' },
      { label: 'Activity', href: '#/activity', description: 'Cross-protocol RF timeline' },
      { label: 'Recording', href: '#/recording', description: 'IQ capture, transcription, annotations' },
    ],
  },
  {
    id: 'signals',
    label: 'Signals',
    items: [
      { label: 'Messages', href: '#/messages', description: 'Full decoded message log' },
      { label: 'Occupancy', href: '#/occupancy', description: 'Band-use heatmap and utilization' },
      { label: 'Trunking', href: '#/trunking', description: 'P25, NXDN, EDACS, DMR' },
      { label: 'Satellites', href: '#/satellites', description: 'GOES LRIT products and radiosondes' },
      { label: 'Aero', href: '#/aero', description: 'Inmarsat AERO' },
      { label: 'Iridium', href: '#/iridium', description: 'Iridium burst decoder' },
      { label: 'HD Radio', href: '#/hd-radio', description: 'NRSC5 FM HD Radio' },
      { label: 'BLE', href: '#/ble', description: 'Bluetooth Low Energy' },
      { label: 'LoRa', href: '#/lora', description: 'LoRa regional plans' },
      { label: 'Signal ID', href: '#/signal-id', description: 'Fingerprints and burst segmentation' },
    ],
  },
  {
    id: 'setup',
    label: 'Setup',
    items: [
      { label: 'Settings', href: '#/settings', description: 'Device, receiver, scanner, audio' },
      { label: 'Profiles', href: '#/profiles', description: 'Hardware profiles and bookmarks' },
      { label: 'Blacklist', href: '#/blacklist', description: 'Frequency exclusion list' },
      { label: 'Cases', href: '#/cases', description: 'Investigation case management' },
      { label: 'Jobs', href: '#/jobs', description: 'Scheduled scan and recording jobs' },
      { label: 'Aircraft', href: '#/aircraft', description: 'ICAO, registration, callsign lookup' },
      { label: 'Lookups', href: '#/lookups', description: 'RadioReference and FCC lookup' },
      { label: 'Feature packs', href: '#/feature-packs', description: 'Enable optional decoder packs' },
      { label: 'Dependencies', href: '#/deps', description: 'Decoder runtime and install status' },
      { label: 'Debug', href: '#/debug', description: 'Stats, logs, classifier state' },
    ],
  },
];

/** All navigable destinations for the command palette. */
export const allNavItems: NavItem[] = navSections.flatMap((section) => section.items);

/** Normalize hash route for active-state comparison. */
export function normalizeRoute(hashOrPath: string): string {
  const raw = hashOrPath.startsWith('#') ? hashOrPath.slice(1) : hashOrPath;
  if (!raw || raw === '/') return '/';
  return raw.startsWith('/') ? raw : `/${raw}`;
}

export function isRouteActive(current: string, href: string): boolean {
  return normalizeRoute(current) === normalizeRoute(href.replace('#', ''));
}

export function navItemForRoute(current: string): NavItem | undefined {
  return allNavItems.find((item) => isRouteActive(current, item.href));
}

export function sectionIdForRoute(current: string): string | null {
  for (const section of navSections) {
    if (section.items.some((item) => isRouteActive(current, item.href))) {
      return section.id;
    }
  }
  return null;
}
