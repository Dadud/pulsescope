/** Main-thread waterfall history renderer. OffscreenCanvas workers are opt-in only
 *  because several desktop webviews fail to display transferred surfaces.
 *
 *  Rows are retained so the operator can zoom and pan after the fact: each row keeps
 *  the capture window it was received under, so history stays frequency-correct
 *  across zoom changes and hardware retunes. */

import {
  getWaterfallLUT,
  normalizeDb,
  type WaterfallPalette,
  waterfallColor,
} from './spectrum-display';

export type { WaterfallPalette };

/** Stored resolution per row and the row cap. Both bound memory: worst case is
 *  ~1600 rows x 2048 floats = 13 MB, and rows beyond the tallest canvas are
 *  never visible. */
const HISTORY_COLUMNS = 2048;
const MAX_HISTORY_ROWS = 1600;

export function canvasBackingSize(
  canvas: HTMLCanvasElement,
  fallbackWidth: number,
  fallbackHeight: number,
) {
  const rect = canvas.getBoundingClientRect();
  const scale = Math.max(1, Math.min(2, window.devicePixelRatio || 1));
  const layoutWidth = rect.width > 0 ? rect.width : fallbackWidth;
  const layoutHeight = rect.height > 0 ? rect.height : fallbackHeight;
  return {
    width: Math.max(320, Math.round(layoutWidth * scale)),
    height: Math.max(80, Math.round(layoutHeight * scale)),
  };
}

export type WaterfallDrawOptions = {
  gain?: number;
  palette?: WaterfallPalette;
  minDb?: number;
  maxDb?: number;
  rowsPerFrame?: number;
  /** Capture window the bins cover. Defaults to the visible window. */
  captureCenterHz?: number;
  captureSpanHz?: number;
  /** Visible window. Defaults to the whole capture window. */
  viewStartHz?: number;
  viewEndHz?: number;
};

type HistoryRow = {
  centerHz: number;
  spanHz: number;
  bins: Float32Array;
};

const CLASSIC_LUT = (() => {
  const lut = new Uint8ClampedArray(256 * 3);
  for (let i = 0; i < 256; i += 1) {
    const rgb = waterfallColor('classic', i / 255);
    lut[i * 3] = Math.round(rgb[0] * 255);
    lut[i * 3 + 1] = Math.round(rgb[1] * 255);
    lut[i * 3 + 2] = Math.round(rgb[2] * 255);
  }
  return lut;
})();

function paletteTable(palette: WaterfallPalette): Uint8ClampedArray {
  return getWaterfallLUT(palette) ?? CLASSIC_LUT;
}

/** Peak-preserving downsample into the stored row width. */
function packRow(bins: number[]): Float32Array {
  const columns = Math.min(HISTORY_COLUMNS, Math.max(1, bins.length));
  const row = new Float32Array(columns);
  if (columns === bins.length) {
    row.set(bins);
    return row;
  }
  for (let i = 0; i < columns; i += 1) {
    const from = Math.floor((i * bins.length) / columns);
    const to = Math.max(from + 1, Math.floor(((i + 1) * bins.length) / columns));
    let peak = -Infinity;
    for (let j = from; j < to && j < bins.length; j += 1) {
      if (bins[j] > peak) peak = bins[j];
    }
    row[i] = Number.isFinite(peak) ? peak : bins[Math.min(bins.length - 1, from)];
  }
  return row;
}

export class WaterfallCanvas {
  private canvas: HTMLCanvasElement | null = null;
  private ctx: CanvasRenderingContext2D | null = null;
  private pixels: Uint8ClampedArray | null = null;
  private image: ImageData | null = null;
  private observer: ResizeObserver | null = null;
  private history: HistoryRow[] = [];
  private options: WaterfallDrawOptions = {};
  private renderKey = '';

  attach(canvas: HTMLCanvasElement) {
    this.detach();
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d');
    this.pixels = null;
    this.image = null;
    this.renderKey = '';
    this.observer = new ResizeObserver(() => {
      this.pixels = null;
      this.image = null;
      this.renderKey = '';
      this.redraw();
    });
    this.observer.observe(canvas);
    this.redraw();
  }

  detach() {
    this.observer?.disconnect();
    this.observer = null;
    this.canvas = null;
    this.ctx = null;
    this.pixels = null;
    this.image = null;
  }

  clear() {
    this.history = [];
    this.renderKey = '';
    if (!this.canvas || !this.ctx) return;
    this.pixels = null;
    this.image = null;
    this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
  }

  /** Re-render retained rows without consuming a new frame. Use after a zoom or
   *  pan so history reflows to the new window. */
  redraw(options?: WaterfallDrawOptions) {
    if (options) this.options = options;
    if (!this.history.length) return;
    this.renderAll();
  }

  draw(
    bins: number[],
    gainOrOptions: number | WaterfallDrawOptions = 1,
    palette?: WaterfallPalette,
  ) {
    const options: WaterfallDrawOptions =
      typeof gainOrOptions === 'number'
        ? { gain: gainOrOptions, palette: palette ?? 'openwebrx' }
        : gainOrOptions;
    this.options = options;
    if (bins.length === 0) return;

    const geometry = this.geometry(options);
    this.history.unshift({
      centerHz: geometry.captureCenterHz,
      spanHz: geometry.captureSpanHz,
      bins: packRow(bins),
    });
    if (this.history.length > MAX_HISTORY_ROWS) this.history.length = MAX_HISTORY_ROWS;

    if (!this.canvas || !this.ctx) return;
    const key = this.keyFor(options);
    if (key === this.renderKey && this.pixels) this.appendRow(options);
    else this.renderAll();
  }

  /** Capture/view window for a draw call, tolerating callers that only pass levels. */
  private geometry(options: WaterfallDrawOptions) {
    const captureSpanHz = Math.max(1, options.captureSpanHz ?? 0);
    const hasCapture = (options.captureSpanHz ?? 0) > 0;
    const captureCenterHz = options.captureCenterHz ?? 0;
    const viewStartHz = hasCapture
      ? (options.viewStartHz ?? captureCenterHz - captureSpanHz / 2)
      : 0;
    const viewEndHz = hasCapture
      ? (options.viewEndHz ?? captureCenterHz + captureSpanHz / 2)
      : captureSpanHz;
    return { hasCapture, captureCenterHz, captureSpanHz, viewStartHz, viewEndHz };
  }

  private keyFor(options: WaterfallDrawOptions): string {
    const geometry = this.geometry(options);
    const w = this.canvas?.width ?? 0;
    const h = this.canvas?.height ?? 0;
    return [
      w,
      h,
      geometry.viewStartHz,
      geometry.viewEndHz,
      geometry.captureCenterHz,
      geometry.captureSpanHz,
      options.minDb ?? -120,
      options.maxDb ?? -20,
      options.gain ?? 1,
      options.palette ?? 'openwebrx',
    ].join('|');
  }

  private resizeBacking(): { width: number; height: number } | null {
    if (!this.canvas) return null;
    const backing = canvasBackingSize(this.canvas, 1200, 420);
    if (this.canvas.width !== backing.width) this.canvas.width = backing.width;
    if (this.canvas.height !== backing.height) this.canvas.height = backing.height;
    return { width: this.canvas.width, height: this.canvas.height };
  }

  /** Column -> stored-bin index for one row geometry. -1 marks columns the row
   *  never covered (outside its capture window). */
  private columnLut(
    rowStartHz: number,
    rowSpanHz: number,
    columns: number,
    width: number,
    viewStartHz: number,
    viewEndHz: number,
  ): Int32Array {
    const lut = new Int32Array(width);
    const viewSpan = Math.max(1, viewEndHz - viewStartHz);
    for (let x = 0; x < width; x += 1) {
      const hz = viewStartHz + (x / Math.max(1, width - 1)) * viewSpan;
      const fraction = (hz - rowStartHz) / Math.max(1, rowSpanHz);
      if (fraction < 0 || fraction > 1) {
        lut[x] = -1;
        continue;
      }
      lut[x] = Math.min(columns - 1, Math.max(0, Math.round(fraction * (columns - 1))));
    }
    return lut;
  }

  /** Without a capture window the row is simply stretched across the surface,
   *  matching the pre-viewport behaviour for callers that only pass levels. */
  private lutForRow(
    row: HistoryRow,
    width: number,
    geometry: ReturnType<WaterfallCanvas['geometry']>,
  ): Int32Array {
    if (!geometry.hasCapture) {
      return this.columnLut(0, row.bins.length, row.bins.length, width, 0, row.bins.length);
    }
    return this.columnLut(
      row.centerHz - row.spanHz / 2,
      row.spanHz,
      row.bins.length,
      width,
      geometry.viewStartHz,
      geometry.viewEndHz,
    );
  }

  private renderAll() {
    const size = this.resizeBacking();
    if (!this.canvas || !this.ctx || !size) return;
    const { width: w, height: h } = size;
    const options = this.options;
    const geometry = this.geometry(options);
    const minDb = options.minDb ?? -120;
    const maxDb = options.maxDb ?? -20;
    const gain = options.gain ?? 1;
    const table = paletteTable(options.palette ?? 'openwebrx');

    if (!this.pixels || this.pixels.length !== w * h * 4) {
      this.pixels = new Uint8ClampedArray(w * h * 4);
    }
    this.pixels.fill(0);

    const lutCache = new Map<string, Int32Array>();
    const rows = Math.min(h, this.history.length);
    for (let y = 0; y < rows; y += 1) {
      const row = this.history[y];
      const cacheKey = `${row.centerHz}:${row.spanHz}:${row.bins.length}`;
      let lut = lutCache.get(cacheKey);
      if (!lut) {
        lut = this.lutForRow(row, w, geometry);
        lutCache.set(cacheKey, lut);
      }
      const rowOffset = y * w * 4;
      for (let x = 0; x < w; x += 1) {
        const index = lut[x];
        const pixel = rowOffset + x * 4;
        if (index < 0) {
          this.pixels[pixel + 3] = 255;
          continue;
        }
        const normalized = Math.max(
          0,
          Math.min(1, normalizeDb(row.bins[index], minDb, maxDb) * gain),
        );
        const color = Math.round(normalized * 255) * 3;
        this.pixels[pixel] = table[color];
        this.pixels[pixel + 1] = table[color + 1];
        this.pixels[pixel + 2] = table[color + 2];
        this.pixels[pixel + 3] = 255;
      }
    }
    for (let y = rows; y < h; y += 1) {
      const rowOffset = y * w * 4;
      for (let x = 0; x < w; x += 1) this.pixels[rowOffset + x * 4 + 3] = 255;
    }

    this.blit(w, h);
    this.renderKey = this.keyFor(options);
  }

  /** Scroll the existing image down and paint only the newest row. */
  private appendRow(options: WaterfallDrawOptions) {
    const size = this.resizeBacking();
    if (!this.canvas || !this.ctx || !size || !this.pixels) return;
    const { width: w, height: h } = size;
    if (this.pixels.length !== w * h * 4) {
      this.renderAll();
      return;
    }
    const row = this.history[0];
    if (!row) return;
    const geometry = this.geometry(options);
    const minDb = options.minDb ?? -120;
    const maxDb = options.maxDb ?? -20;
    const gain = options.gain ?? 1;
    const table = paletteTable(options.palette ?? 'openwebrx');
    const rowsPerFrame = Math.max(1, Math.min(4, options.rowsPerFrame ?? 1));

    const rowBytes = w * 4;
    if (h > rowsPerFrame) {
      this.pixels.copyWithin(rowBytes * rowsPerFrame, 0, rowBytes * (h - rowsPerFrame));
    }

    const lut = this.lutForRow(row, w, geometry);

    for (let x = 0; x < w; x += 1) {
      const pixel = x * 4;
      const index = lut[x];
      if (index < 0) {
        this.pixels[pixel] = 0;
        this.pixels[pixel + 1] = 0;
        this.pixels[pixel + 2] = 0;
        this.pixels[pixel + 3] = 255;
        continue;
      }
      const normalized = Math.max(
        0,
        Math.min(1, normalizeDb(row.bins[index], minDb, maxDb) * gain),
      );
      const color = Math.round(normalized * 255) * 3;
      this.pixels[pixel] = table[color];
      this.pixels[pixel + 1] = table[color + 1];
      this.pixels[pixel + 2] = table[color + 2];
      this.pixels[pixel + 3] = 255;
    }
    for (let r = 1; r < rowsPerFrame; r += 1) {
      this.pixels.set(this.pixels.subarray(0, rowBytes), r * rowBytes);
    }

    this.blit(w, h);
  }

  private blit(w: number, h: number) {
    if (!this.ctx || !this.pixels) return;
    if (!this.image || this.image.width !== w || this.image.height !== h) {
      this.image = this.ctx.createImageData(w, h);
    }
    this.image.data.set(this.pixels);
    this.ctx.putImageData(this.image, 0, 0);
  }
}
