/** Main-thread waterfall history renderer. OffscreenCanvas workers are opt-in only
 *  because several desktop webviews fail to display transferred surfaces. */

import {
  getWaterfallLUT,
  normalizeDb,
  sampleBinLinear,
  type WaterfallPalette,
  waterfallColor,
} from './spectrum-display';

export type { WaterfallPalette };

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
};

export class WaterfallCanvas {
  private canvas: HTMLCanvasElement | null = null;
  private ctx: CanvasRenderingContext2D | null = null;
  private pixels: Uint8ClampedArray | null = null;
  private image: ImageData | null = null;
  private observer: ResizeObserver | null = null;
  private lastBins: number[] = [];
  private options: WaterfallDrawOptions = {};

  attach(canvas: HTMLCanvasElement) {
    this.detach();
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d');
    this.pixels = null;
    this.image = null;
    this.observer = new ResizeObserver(() => {
      this.pixels = null;
      this.image = null;
      if (this.lastBins.length) this.draw(this.lastBins, this.options);
    });
    this.observer.observe(canvas);
    if (this.lastBins.length) this.draw(this.lastBins, this.options);
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
    this.lastBins = [];
    if (!this.canvas || !this.ctx) return;
    this.pixels = null;
    this.image = null;
    this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
  }

  draw(bins: number[], gainOrOptions: number | WaterfallDrawOptions = 1, palette?: WaterfallPalette) {
    const options: WaterfallDrawOptions =
      typeof gainOrOptions === 'number'
        ? { gain: gainOrOptions, palette: palette ?? 'openwebrx' }
        : gainOrOptions;
    this.lastBins = bins;
    this.options = options;
    if (!this.canvas || !this.ctx || bins.length === 0) return;

    const minDb = options.minDb ?? -120;
    const maxDb = options.maxDb ?? -20;
    const gain = options.gain ?? 1;
    const paletteName = options.palette ?? 'openwebrx';
    const rowsPerFrame = Math.max(1, Math.min(4, options.rowsPerFrame ?? 1));
    const lut = getWaterfallLUT(paletteName);

    const backing = canvasBackingSize(this.canvas, 1200, 420);
    if (this.canvas.width !== backing.width) this.canvas.width = backing.width;
    if (this.canvas.height !== backing.height) this.canvas.height = backing.height;

    const w = this.canvas.width;
    const h = this.canvas.height;
    const ctx = this.ctx;

    if (!this.pixels || this.pixels.length !== w * h * 4) {
      this.pixels = new Uint8ClampedArray(w * h * 4);
    }

    const rowBytes = w * 4;
    if (h > rowsPerFrame) {
      this.pixels.copyWithin(rowBytes * rowsPerFrame, 0, rowBytes * (h - rowsPerFrame));
    }

    for (let x = 0; x < w; x += 1) {
      const db = sampleBinLinear(bins, x / Math.max(1, w - 1));
      const normalized = Math.max(0, Math.min(1, normalizeDb(db, minDb, maxDb) * gain));
      const rgb = waterfallColor(paletteName, normalized, lut ?? undefined);
      const pixel = x * 4;
      this.pixels[pixel] = Math.round(rgb[0] * 255);
      this.pixels[pixel + 1] = Math.round(rgb[1] * 255);
      this.pixels[pixel + 2] = Math.round(rgb[2] * 255);
      this.pixels[pixel + 3] = 255;
    }

    for (let row = 1; row < rowsPerFrame; row += 1) {
      this.pixels.set(this.pixels.subarray(0, rowBytes), row * rowBytes);
    }

    if (!this.image || this.image.width !== w || this.image.height !== h) {
      this.image = ctx.createImageData(w, h);
    }
    this.image.data.set(this.pixels);
    ctx.putImageData(this.image, 0, 0);
  }
}
