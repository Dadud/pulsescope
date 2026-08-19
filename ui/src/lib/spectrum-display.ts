/** Shared spectrum/waterfall display math — openWebRX-inspired levels, colormaps, and smoothing. */

export type WaterfallPalette = 'openwebrx' | 'classic' | 'mono';

export type SpectrumDisplayConfig = {
  minDb: number;
  maxDb: number;
  smoothing: number;
  peakHold: boolean;
  palette: WaterfallPalette;
};

export const DEFAULT_SPECTRUM_DISPLAY: SpectrumDisplayConfig = {
  minDb: -120,
  maxDb: -20,
  smoothing: 0.35,
  peakHold: true,
  palette: 'openwebrx',
};

const STORAGE_KEY = 'pulsescope.spectrum.display';

export function loadSpectrumDisplayConfig(): SpectrumDisplayConfig {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULT_SPECTRUM_DISPLAY };
    const parsed = JSON.parse(raw) as Partial<SpectrumDisplayConfig>;
    const config: SpectrumDisplayConfig = {
      minDb: clampDb(parsed.minDb ?? DEFAULT_SPECTRUM_DISPLAY.minDb),
      maxDb: clampDb(parsed.maxDb ?? DEFAULT_SPECTRUM_DISPLAY.maxDb),
      smoothing: clamp01(parsed.smoothing ?? DEFAULT_SPECTRUM_DISPLAY.smoothing),
      peakHold: parsed.peakHold ?? DEFAULT_SPECTRUM_DISPLAY.peakHold,
      palette: paletteFromString(parsed.palette),
    };
    if (config.maxDb <= config.minDb + 10) {
      return { ...DEFAULT_SPECTRUM_DISPLAY };
    }
    return config;
  } catch {
    return { ...DEFAULT_SPECTRUM_DISPLAY };
  }
}

export function saveSpectrumDisplayConfig(config: SpectrumDisplayConfig) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
}

function clampDb(value: number): number {
  return Math.max(-160, Math.min(20, value));
}

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

function paletteFromString(value: unknown): WaterfallPalette {
  if (value === 'classic' || value === 'mono' || value === 'openwebrx') return value;
  return DEFAULT_SPECTRUM_DISPLAY.palette;
}

export function normalizeDb(db: number, minDb: number, maxDb: number): number {
  const span = Math.max(1, maxDb - minDb);
  return Math.max(0, Math.min(1, (db - minDb) / span));
}

/** Narrowest span the display will zoom to, and the deepest zoom relative to the
 *  capture window. FFT bins do not gain resolution past the capture bin width, so
 *  zooming further would only interpolate. */
export const MIN_VIEW_SPAN_HZ = 2_000;
export const MAX_VIEW_ZOOM = 128;

/** Frequency window the operator is looking at. Always a subset of the hardware
 *  capture window; the server still delivers the full captured span. */
export type SpectrumViewport = { centerHz: number; spanHz: number };

export function clampViewport(
  view: SpectrumViewport,
  captureCenterHz: number,
  captureSpanHz: number,
): SpectrumViewport {
  const capture = Math.max(1, captureSpanHz);
  const minSpan = Math.min(capture, Math.max(MIN_VIEW_SPAN_HZ, capture / MAX_VIEW_ZOOM));
  const spanHz = Math.max(minSpan, Math.min(capture, view.spanHz > 0 ? view.spanHz : capture));
  const half = spanHz / 2;
  const low = captureCenterHz - capture / 2 + half;
  const high = captureCenterHz + capture / 2 - half;
  const requested = view.centerHz > 0 ? view.centerHz : captureCenterHz;
  return { centerHz: Math.min(high, Math.max(low, requested)), spanHz };
}

/** Zoom by `factor` keeping the frequency under `anchorFraction` (0..1 across the
 *  view) pinned, so wheel zoom tracks the cursor. */
export function zoomViewportAt(
  view: SpectrumViewport,
  factor: number,
  anchorFraction: number,
  captureCenterHz: number,
  captureSpanHz: number,
): SpectrumViewport {
  const anchor = Math.max(0, Math.min(1, anchorFraction));
  const anchorHz = view.centerHz - view.spanHz / 2 + anchor * view.spanHz;
  const target = clampViewport(
    { centerHz: view.centerHz, spanHz: view.spanHz / Math.max(0.01, factor) },
    captureCenterHz,
    captureSpanHz,
  );
  return clampViewport(
    { centerHz: anchorHz - (anchor - 0.5) * target.spanHz, spanHz: target.spanHz },
    captureCenterHz,
    captureSpanHz,
  );
}

export function viewportZoom(view: SpectrumViewport, captureSpanHz: number): number {
  return Math.max(1, captureSpanHz) / Math.max(1, view.spanHz);
}

export function viewportFrequencyAt(view: SpectrumViewport, fraction: number): number {
  return view.centerHz - view.spanHz / 2 + Math.max(0, Math.min(1, fraction)) * view.spanHz;
}

export function viewportFractionOf(view: SpectrumViewport, hz: number): number {
  return (hz - (view.centerHz - view.spanHz / 2)) / Math.max(1, view.spanHz);
}

/** Position of `hz` inside the captured FFT bin array, as a 0..1 fraction. */
export function captureFractionOf(hz: number, captureCenterHz: number, captureSpanHz: number): number {
  const capture = Math.max(1, captureSpanHz);
  return (hz - (captureCenterHz - capture / 2)) / capture;
}

export function formatZoom(zoom: number): string {
  if (zoom < 9.95) return `${zoom.toFixed(1)}×`;
  return `${Math.round(zoom)}×`;
}

export function formatSpan(spanHz: number): string {
  if (spanHz >= 1e6) return `${(spanHz / 1e6).toFixed(spanHz >= 1e7 ? 1 : 3)} MHz`;
  return `${Math.round(spanHz / 1e3)} kHz`;
}

export function sampleBinLinear(bins: number[], fraction: number): number {
  if (!bins.length) return minDbFallback();
  const clamped = Math.max(0, Math.min(1, fraction));
  const position = clamped * (bins.length - 1);
  const left = Math.floor(position);
  const right = Math.min(bins.length - 1, left + 1);
  const mix = position - left;
  return bins[left] * (1 - mix) + bins[right] * mix;
}

function minDbFallback(): number {
  return DEFAULT_SPECTRUM_DISPLAY.minDb;
}

export function autoDisplayLevels(
  bins: number[],
  marginMin = 5,
  marginMax = 10,
  minRange = 50,
): { minDb: number; maxDb: number } {
  let measuredMin = Infinity;
  let measuredMax = -Infinity;
  for (const value of bins) {
    if (!Number.isFinite(value)) continue;
    measuredMin = Math.min(measuredMin, value);
    measuredMax = Math.max(measuredMax, value);
  }
  if (!Number.isFinite(measuredMin) || !Number.isFinite(measuredMax)) {
    return { minDb: DEFAULT_SPECTRUM_DISPLAY.minDb, maxDb: DEFAULT_SPECTRUM_DISPLAY.maxDb };
  }
  let minDb = measuredMin - marginMin;
  let maxDb = measuredMax + marginMax;
  if (maxDb - minDb < minRange) {
    const center = (maxDb + minDb) / 2;
    minDb = center - minRange / 2;
    maxDb = center + minRange / 2;
  }
  minDb = clampDb(minDb);
  maxDb = clampDb(Math.max(minDb + 10, maxDb));
  return { minDb, maxDb };
}

export class SpectrumSmoother {
  private history: number[] | null = null;

  constructor(private alpha = 0.35) {}

  setAlpha(alpha: number) {
    this.alpha = clamp01(alpha);
  }

  process(bins: number[]): number[] {
    if (!bins.length) return bins;
    if (!this.history || this.history.length !== bins.length) {
      this.history = bins.slice();
      return this.history;
    }
    const keep = 1 - this.alpha;
    for (let i = 0; i < bins.length; i += 1) {
      this.history[i] = this.history[i] * keep + bins[i] * this.alpha;
    }
    return this.history;
  }

  reset() {
    this.history = null;
  }
}

export class PeakHoldTrace {
  private peaks: number[] | null = null;
  private lastMs = 0;

  constructor(private decayDbPerMs = 0.012) {}

  process(bins: number[], nowMs = Date.now()): number[] {
    if (!bins.length) return bins;
    const elapsed = this.lastMs > 0 ? Math.max(0, nowMs - this.lastMs) : 0;
    this.lastMs = nowMs;
    const decay = this.decayDbPerMs * elapsed;
    if (!this.peaks || this.peaks.length !== bins.length) {
      this.peaks = bins.slice();
      return this.peaks;
    }
    for (let i = 0; i < bins.length; i += 1) {
      if (bins[i] >= this.peaks[i]) this.peaks[i] = bins[i];
      else this.peaks[i] = Math.max(bins[i], this.peaks[i] - decay);
    }
    return this.peaks;
  }

  reset() {
    this.peaks = null;
    this.lastMs = 0;
  }
}

type Rgb = [number, number, number];

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function lerpRgb(a: Rgb, b: Rgb, t: number): Rgb {
  return [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)];
}

function buildGradientLUT(stops: Array<{ t: number; rgb: Rgb }>): Uint8ClampedArray {
  const lut = new Uint8ClampedArray(256 * 3);
  for (let i = 0; i < 256; i += 1) {
    const t = i / 255;
    let left = stops[0];
    for (let s = 1; s < stops.length; s += 1) {
      if (stops[s].t >= t) {
        const right = stops[s];
        const span = right.t - left.t;
        const mix = span > 0 ? (t - left.t) / span : 0;
        const rgb = lerpRgb(left.rgb, right.rgb, mix);
        const offset = i * 3;
        lut[offset] = Math.round(rgb[0] * 255);
        lut[offset + 1] = Math.round(rgb[1] * 255);
        lut[offset + 2] = Math.round(rgb[2] * 255);
        break;
      }
      left = stops[s];
    }
  }
  return lut;
}

const OPENWEBRX_LUT = buildGradientLUT([
  { t: 0, rgb: [0, 0, 0] },
  { t: 0.12, rgb: [0, 0, 0.45] },
  { t: 0.28, rgb: [0, 0.55, 1] },
  { t: 0.42, rgb: [0, 1, 1] },
  { t: 0.55, rgb: [0, 1, 0] },
  { t: 0.68, rgb: [1, 1, 0] },
  { t: 0.82, rgb: [1, 0, 0] },
  { t: 0.92, rgb: [1, 0, 1] },
  { t: 1, rgb: [1, 1, 1] },
]);

const MONO_LUT = buildGradientLUT([
  { t: 0, rgb: [0, 0, 0] },
  { t: 1, rgb: [1, 1, 1] },
]);

function hsvToRgb(hue: number, value: number): Rgb {
  const c = 1 - Math.abs((hue / 60) % 2 - 1);
  const sector = Math.floor(hue / 60);
  switch (sector) {
    case 0: return [1, c, 0];
    case 1: return [c, 1, 0];
    case 2: return [0, 1, c];
    case 3: return [0, c, 1];
    case 4: return [c, 0, 1];
    default: return [1, 0, c];
  }
}

export function waterfallColor(
  palette: WaterfallPalette,
  normalized: number,
  lut?: Uint8ClampedArray,
): [number, number, number] {
  const value = Math.max(0, Math.min(1, normalized));
  if (palette === 'mono') {
    const table = lut ?? MONO_LUT;
    const index = Math.round(value * 255) * 3;
    return [table[index] / 255, table[index + 1] / 255, table[index + 2] / 255];
  }
  if (palette === 'openwebrx') {
    const table = lut ?? OPENWEBRX_LUT;
    const index = Math.round(value * 255) * 3;
    return [table[index] / 255, table[index + 1] / 255, table[index + 2] / 255];
  }
  const hue = 240 - value * 240;
  const rgb = hsvToRgb(hue, 1);
  return [rgb[0] * value, rgb[1] * value, rgb[2] * value];
}

export function getWaterfallLUT(palette: WaterfallPalette): Uint8ClampedArray | null {
  if (palette === 'openwebrx') return OPENWEBRX_LUT;
  if (palette === 'mono') return MONO_LUT;
  return null;
}

export function formatSpectrumFrequency(hz: number): string {
  if (hz >= 1e9) return `${(hz / 1e9).toFixed(4)}G`;
  return `${(hz / 1e6).toFixed(4)}M`;
}

export function horizontalDbGridLines(minDb: number, maxDb: number, count = 6): number[] {
  const lines: number[] = [];
  const step = (maxDb - minDb) / count;
  for (let i = 0; i <= count; i += 1) lines.push(minDb + step * i);
  return lines;
}

export function isCommonRfSetting(setting: { key?: string; name?: string }): boolean {
  const blob = `${setting.key ?? ''} ${setting.name ?? ''}`.toLowerCase();
  return /bias|tee|bt_|_bt|preamp|pre-amp|amp enable|rfnotch|notch|tuner|antenna|lna|ifnotch/.test(blob);
}

export function gainStageLabel(stage: { name: string }, driver?: string): string {
  if (driver === 'sdrplay' && (stage.name === 'IFGR' || stage.name === 'RFGR')) {
    return `${stage.name} gain reduction`;
  }
  return `${stage.name} gain`;
}
