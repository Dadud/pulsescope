type RenderMessage = { canvas?: OffscreenCanvas; bins?: number[]; gain?: number; palette?: 'classic' | 'mono'; width?: number; height?: number; clear?: boolean };

let target: OffscreenCanvas | null = null;
let gl: WebGL2RenderingContext | null = null;
let context2d: OffscreenCanvasRenderingContext2D | null = null;
let intensity: Uint8Array | null = null;
let rgba: Uint8ClampedArray | null = null;
let image: ImageData | null = null;
let texture: WebGLTexture | null = null;
let paletteLocation: WebGLUniformLocation | null = null;
let width = 0;
let height = 0;

function resizeHistory(nextWidth: number, nextHeight: number) {
  if (!target || nextWidth === width && nextHeight === height) return;
  const previous = intensity;
  const previousWidth = width;
  const previousHeight = height;
  width = Math.max(1, nextWidth);
  height = Math.max(1, nextHeight);
  target.width = width;
  target.height = height;
  intensity = new Uint8Array(width * height);
  if (previous && previousWidth > 0 && previousHeight > 0) {
    for (let y = 0; y < height; y += 1) {
      const sourceY = Math.min(previousHeight - 1, Math.floor(y / height * previousHeight));
      for (let x = 0; x < width; x += 1) {
        const sourceX = Math.min(previousWidth - 1, Math.floor(x / width * previousWidth));
        intensity[y * width + x] = previous[sourceY * previousWidth + sourceX];
      }
    }
  }
  if (gl) gl.viewport(0, 0, width, height);
  if (context2d) {
    rgba = new Uint8ClampedArray(width * height * 4);
    image = context2d.createImageData(width, height);
  }
}

function compileShader(context: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = context.createShader(type)!;
  context.shaderSource(shader, source);
  context.compileShader(shader);
  if (!context.getShaderParameter(shader, context.COMPILE_STATUS)) throw new Error(context.getShaderInfoLog(shader) ?? 'shader compile failed');
  return shader;
}

function initWebGl(canvas: OffscreenCanvas): boolean {
  try {
    gl = canvas.getContext('webgl2', { antialias: false, depth: false, preserveDrawingBuffer: false });
    if (!gl) return false;
    const vertex = compileShader(gl, gl.VERTEX_SHADER, `#version 300 es
      in vec2 position; out vec2 uv;
      void main(){ uv = vec2((position.x + 1.0) * .5, (1.0 - position.y) * .5); gl_Position = vec4(position, 0.0, 1.0); }`);
    const fragment = compileShader(gl, gl.FRAGMENT_SHADER, `#version 300 es
      precision mediump float; uniform sampler2D history; uniform int mono; in vec2 uv; out vec4 color;
      vec3 heat(float v){ float h=(1.0-v)*4.0; float x=1.0-abs(mod(h,2.0)-1.0); return h<1.0?vec3(1.0,x,0.0):h<2.0?vec3(x,1.0,0.0):h<3.0?vec3(0.0,1.0,x):vec3(0.0,x,1.0); }
      void main(){ float v=texture(history,uv).r; color=vec4(mono==1?vec3(v):heat(v)*v,1.0); }`);
    const program = gl.createProgram()!;
    gl.attachShader(program, vertex); gl.attachShader(program, fragment); gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? 'program link failed');
    gl.useProgram(program);
    const position = gl.getAttribLocation(program, 'position');
    const buffer = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1, 1,-1, -1,1, -1,1, 1,-1, 1,1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(position); gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
    texture = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    paletteLocation = gl.getUniformLocation(program, 'mono');
    gl.viewport(0, 0, width, height);
    return true;
  } catch {
    gl = null; texture = null; paletteLocation = null;
    return false;
  }
}

function updateHistory(bins: number[], gain: number) {
  if (!intensity) return;
  const rows = 3;
  intensity.copyWithin(width * rows, 0, width * (height - rows));
  for (let x = 0; x < width; x += 1) {
    const index = Math.min(bins.length - 1, Math.floor((x / width) * bins.length));
    intensity[x] = Math.round(Math.max(0, Math.min(1, ((bins[index] + 100) / 80) * gain)) * 255);
  }
  for (let row = 1; row < rows; row += 1) intensity.set(intensity.subarray(0, width), row * width);
}

function render2d(palette: 'classic' | 'mono') {
  if (!context2d || !intensity || !rgba || !image) return;
  for (let i = 0; i < intensity.length; i += 1) {
    const value = intensity[i] / 255; const hue = 240 - value * 240; const c = 1 - Math.abs((hue / 60) % 2 - 1); const sector = Math.floor(hue / 60);
    const rgb = palette === 'mono' ? [1,1,1] : sector===0?[1,c,0]:sector===1?[c,1,0]:sector===2?[0,1,c]:sector===3?[0,c,1]:[c,0,1];
    rgba[i*4]=Math.round(rgb[0]*value*255); rgba[i*4+1]=Math.round(rgb[1]*value*255); rgba[i*4+2]=Math.round(rgb[2]*value*255); rgba[i*4+3]=255;
  }
  image.data.set(rgba); context2d.putImageData(image, 0, 0);
}

self.onmessage = (event: MessageEvent<RenderMessage>) => {
  const message = event.data;
  if (message.canvas) {
    target = message.canvas; width = message.width ?? target.width; height = message.height ?? target.height; target.width = width; target.height = height;
    intensity = new Uint8Array(width * height);
    const renderer = initWebGl(target) ? 'webgl2' : 'canvas2d';
    if (!gl) { context2d = target.getContext('2d'); rgba = new Uint8ClampedArray(width * height * 4); image = context2d?.createImageData(width, height) ?? null; }
    self.postMessage({ type: 'ready', renderer }); return;
  }
  if (target && message.width && message.height) resizeHistory(message.width, message.height);
  if (message.clear && intensity) intensity.fill(0);
  if (message.bins?.length) updateHistory(message.bins, message.gain ?? 1);
  if (!intensity || (!message.bins?.length && !message.clear)) return;
  const palette = message.palette ?? 'classic';
  if (gl && texture) {
    gl.bindTexture(gl.TEXTURE_2D, texture); gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, width, height, 0, gl.RED, gl.UNSIGNED_BYTE, intensity);
    gl.uniform1i(paletteLocation, palette === 'mono' ? 1 : 0); gl.drawArrays(gl.TRIANGLES, 0, 6);
  } else render2d(palette);
};
