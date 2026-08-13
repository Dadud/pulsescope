type RenderMessage = {
  canvas?: OffscreenCanvas;
  bins?: number[];
  gain?: number;
  palette?: 'classic' | 'mono';
  width?: number;
  height?: number;
  clear?: boolean;
};

let target: OffscreenCanvas | null = null;
let context: OffscreenCanvasRenderingContext2D | null = null;
let pixels: Uint8ClampedArray | null = null;
let image: ImageData | null = null;
let width = 0;
let height = 0;

self.onmessage = (event: MessageEvent<RenderMessage>) => {
  const message = event.data;
  if (message.canvas) {
    target = message.canvas;
    width = message.width ?? target.width;
    height = message.height ?? target.height;
    target.width = width;
    target.height = height;
    context = target.getContext('2d');
    pixels = new Uint8ClampedArray(width * height * 4);
    image = context?.createImageData(width, height) ?? null;
    return;
  }
  if (message.clear && context && pixels && image) {
    pixels.fill(0);
    image.data.set(pixels);
    context.putImageData(image, 0, 0);
    return;
  }
  if (!context || !pixels || !image || !message.bins?.length) return;
  const bins = message.bins;
  const gain = message.gain ?? 1;
  const palette = message.palette ?? 'classic';
  const rowBytes = width * 4;
  const rowsPerFrame = 3;
  if (height > rowsPerFrame) pixels.copyWithin(rowBytes * rowsPerFrame, 0, rowBytes * (height - rowsPerFrame));
  for (let x = 0; x < width; x += 1) {
    const index = Math.min(bins.length - 1, Math.floor((x / width) * bins.length));
    const value = Math.max(0, Math.min(1, ((bins[index] + 100) / 80) * gain));
    const hue = 240 - value * 240;
    const c = 1 - Math.abs((hue / 60) % 2 - 1);
    const sector = Math.floor(hue / 60);
    const rgb = palette === 'mono'
      ? [1, 1, 1]
      : sector === 0 ? [1, c, 0] : sector === 1 ? [c, 1, 0] : sector === 2 ? [0, 1, c] : sector === 3 ? [0, c, 1] : sector === 4 ? [c, 0, 1] : [1, 0, c];
    const pixel = x * 4;
    pixels[pixel] = Math.round(rgb[0] * value * 255);
    pixels[pixel + 1] = Math.round(rgb[1] * value * 255);
    pixels[pixel + 2] = Math.round(rgb[2] * value * 255);
    pixels[pixel + 3] = 255;
  }
  for (let row = 1; row < rowsPerFrame; row += 1) pixels.set(pixels.subarray(0, rowBytes), row * rowBytes);
  image.data.set(pixels);
  context.putImageData(image, 0, 0);
};
