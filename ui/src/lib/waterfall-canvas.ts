/** Main-thread waterfall history renderer. OffscreenCanvas workers are opt-in only
 *  because several desktop webviews fail to display transferred surfaces. */

export type WaterfallPalette = 'classic' | 'mono';

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

function rgbForValue(palette: WaterfallPalette, value: number, hue: number): [number, number, number] {
  if (palette === 'mono') return [1, 1, 1];
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

export class WaterfallCanvas {
  private canvas: HTMLCanvasElement | null = null;
  private ctx: CanvasRenderingContext2D | null = null;
  private pixels: Uint8ClampedArray | null = null;
  private image: ImageData | null = null;
  private observer: ResizeObserver | null = null;
  private lastBins: number[] = [];
  private gain = 1;
  private palette: WaterfallPalette = 'classic';

  attach(canvas: HTMLCanvasElement) {
    this.detach();
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d');
    this.pixels = null;
    this.image = null;
    this.observer = new ResizeObserver(() => {
      this.pixels = null;
      this.image = null;
      if (this.lastBins.length) this.draw(this.lastBins, this.gain, this.palette);
    });
    this.observer.observe(canvas);
    if (this.lastBins.length) this.draw(this.lastBins, this.gain, this.palette);
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

  draw(bins: number[], gain: number, palette: WaterfallPalette) {
    this.lastBins = bins;
    this.gain = gain;
    this.palette = palette;
    if (!this.canvas || !this.ctx || bins.length === 0) return;

    const backing = canvasBackingSize(this.canvas, 1200, 420);
    if (this.canvas.width !== backing.width) this.canvas.width = backing.width;
    if (this.canvas.height !== backing.height) this.canvas.height = backing.height;

    const w = this.canvas.width;
    const h = this.canvas.height;
    const ctx = this.ctx;

    if (!this.pixels || this.pixels.length !== w * h * 4) {
      this.pixels = new Uint8ClampedArray(w * h * 4);
    }

    const rowsPerFrame = 3;
    const rowBytes = w * 4;
    if (h > rowsPerFrame) {
      this.pixels.copyWithin(rowBytes * rowsPerFrame, 0, rowBytes * (h - rowsPerFrame));
    }

    for (let x = 0; x < w; x += 1) {
      const index = Math.min(bins.length - 1, Math.floor((x / w) * bins.length));
      const value = Math.max(0, Math.min(1, ((bins[index] + 100) / 80) * gain));
      const hue = 240 - value * 240;
      const rgb = rgbForValue(palette, value, hue);
      const pixel = x * 4;
      this.pixels[pixel] = Math.round(rgb[0] * value * 255);
      this.pixels[pixel + 1] = Math.round(rgb[1] * value * 255);
      this.pixels[pixel + 2] = Math.round(rgb[2] * value * 255);
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
