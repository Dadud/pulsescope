<script lang="ts">
  import { untrack } from 'svelte';
  import { Api, openEvents, openSpectrum, type ScanRange, type VfoState, type DecodedMessage, type ScannerEvent, type SpectrumStreamFrame } from '$lib/api';
  import { BrowserAudio, type BrowserAudioState } from '$lib/browser-audio';

  let banks: ScanRange[] = $state([]);
  let activeRange: string | null = $state(null);
  let vfos: VfoState[] = $state([]);
  let messages: DecodedMessage[] = $state([]);
  let signalHistory: any[] = $state([]);
  let spectrumBins: number[] = $state([]);
  let spectrumError = $state('');
  let deviceLabel = $state('—');
  let connected = $state(false);
  let scanRunning = $state(false);
  let centerFreqHz = $state(0);
  let sampleRateHz = $state(1);
  let filter = $state('');
  let messageSearch = $state('');
  let dockFilter = $state('all');
  let notice = $state('');
  let canvas: HTMLCanvasElement;
  let waterfallCanvas: HTMLCanvasElement;
  let ws: WebSocket | null = $state(null);
  let eventReconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let eventConnection = $state<'connecting' | 'open' | 'closed' | 'error'>('connecting');
  let spectrumWs: WebSocket | null = null;
  let spectrumReconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let spectrumConnection = $state<'connecting' | 'open' | 'closed' | 'error'>('connecting');
  let lastSpectrumAt = $state(0);
  let lastSpectrumSequence = $state(0);
  let droppedSpectrumFrames = $state(0);
  let nowMs = $state(Date.now());
  let waterfallPixels: Uint8ClampedArray | null = null;
  let waterfallImage: ImageData | null = null;
  let waterfallWorker: Worker | null = null;
  let drawPending = false;
  let waterfallGain = $state(1);
  let waterfallPalette = $state('classic');
  let initialLoadInFlight = false;
  let browserAudio: BrowserAudio | null = null;
  let audioState: BrowserAudioState = $state('off');
  let audioGesturePending = false;
  let livePollTimer: ReturnType<typeof setTimeout> | null = null;
  let livePollBusy = false;
  let livePollTick = 0;
  let livePolling = false;
  let spectrumTool: 'vfo' | 'center' = $state('vfo');
  let pendingCenterHz: number | null = $state(null);
  let centerTuneTimer: ReturnType<typeof setTimeout> | null = null;
  let centerTuneBusy = $state(false);
  let panPointerId: number | null = null;
  let panStartX = 0;
  let panStartCenterHz = 0;
  let panDragged = false;
  let suppressSurfaceClick = false;

  const spectrumStale = $derived(lastSpectrumAt === 0 || nowMs - lastSpectrumAt > 2_000);
  const displayedCenterHz = $derived(pendingCenterHz ?? centerFreqHz);

  const filteredBanks = $derived(
    banks.filter((b) => b.enabled !== false && b.name.toLowerCase().includes(filter.toLowerCase()))
  );

  const groupedBanks = $derived(
    ['HF', 'VHF', 'UHF', 'Microwave', 'Broadcast', 'Satellite', 'ISM', 'Other']
      .map((group) => ({ group, banks: filteredBanks.filter((bank) => bandGroup(bank) === group) }))
      .filter((entry) => entry.banks.length > 0)
  );

  const visibleMessages = $derived(
    messages.filter((m) => {
      const matchesTab = dockFilter === 'all' || m.protocol.toLowerCase().includes(dockFilter);
      const needle = messageSearch.trim().toLowerCase();
      return matchesTab && (!needle || `${m.protocol} ${m.content}`.toLowerCase().includes(needle));
    })
  );

  const quickModes = [
    { label: 'FM Radio', match: 'FM Broadcast' },
    { label: 'Weather', match: 'NOAA Weather' },
    { label: 'Airband', match: 'Aircraft AM' },
    { label: '2m Amateur', match: '2m Amateur' },
    { label: 'ADS-B 1090', match: 'ADS-B' },
    { label: 'AIS 162', match: 'AIS' },
    { label: 'ACARS 130', match: 'ACARS' },
    { label: 'APRS 144', match: 'APRS' },
    { label: '433 Sensors', match: 'ISM 433' },
    { label: '915 Sensors', match: 'ISM 915' },
    { label: 'Pagers', match: 'Pagers' }
  ];

  function applyVfos(nextVfos: VfoState[]) {
    vfos = nextVfos;
    // A band/profile change intentionally creates a muted VFO. Tear down the
    // old audio subscription instead of continuing to claim "Playing" while
    // the backend has correctly stopped producing frames.
    if (!audioGesturePending && !nextVfos.some((vfo) => !vfo.muted) && audioState !== 'off') {
      browserAudio?.stop();
      audioState = 'off';
    }
  }


  // SvelteKit's hash/static production build eliminated the onMount callback
  // from this route, leaving only a convincing but inert HTML shell. A runes
  // effect is retained in the client bundle; untrack prevents live state read
  // during setup from turning reconnects into effect restarts.
  $effect(() => untrack(() => {
    browserAudio = new BrowserAudio((state) => { audioState = state; });
    waterfallGain = Math.max(0.25, Math.min(4, Number(localStorage.getItem('pulsescope.waterfall.gain') ?? 1)) || 1);
    waterfallPalette = localStorage.getItem('pulsescope.waterfall.palette') === 'mono' ? 'mono' : 'classic';
    const onSpectrum = (event: Event) => {
      const spectrum = (event as CustomEvent).detail;
      activeRange = spectrum?.range ?? activeRange;
      scanRunning = Boolean(spectrum?.running);
      if (Array.isArray(spectrum?.bins) && spectrum.bins.length) {
        spectrumError = '';
        applySpectrum(spectrum.bins);
      }
    };
    const onRuntime = (event: Event) => {
      const { status, vfos: nextVfos } = (event as CustomEvent).detail;
      deviceLabel = status.label; connected = status.connected;
      centerFreqHz = Number(status.center_freq_hz ?? centerFreqHz);
      sampleRateHz = Number(status.sample_rate ?? sampleRateHz); applyVfos(nextVfos);
    };
    const onPollError = (event: Event) => { spectrumError = (event as CustomEvent).detail; };
    window.addEventListener('pulsescope:spectrum', onSpectrum);
    window.addEventListener('pulsescope:runtime', onRuntime);
    window.addEventListener('pulsescope:poll-error', onPollError);
    livePolling = true;
    connectSpectrum();
    scheduleLivePoll(0);
    const onVisibility = () => {
      if (document.visibilityState === 'visible') scheduleLivePoll(0);
    };
    document.addEventListener('visibilitychange', onVisibility);
    void (async () => {
      await loadInitial();
      connectEvents();
    })();
    return () => {
      window.removeEventListener('pulsescope:spectrum', onSpectrum);
      window.removeEventListener('pulsescope:runtime', onRuntime);
      window.removeEventListener('pulsescope:poll-error', onPollError);
      document.removeEventListener('visibilitychange', onVisibility);
      livePolling = false;
      if (livePollTimer) window.clearTimeout(livePollTimer);
      livePollTimer = null;
      if (centerTuneTimer) window.clearTimeout(centerTuneTimer);
      centerTuneTimer = null;
      if (spectrumReconnectTimer) window.clearTimeout(spectrumReconnectTimer);
      spectrumReconnectTimer = null;
      if (eventReconnectTimer) window.clearTimeout(eventReconnectTimer);
      eventReconnectTimer = null;
      spectrumWs?.close(); spectrumWs = null;
      ws?.close(); ws = null;
      waterfallWorker?.terminate(); waterfallWorker = null;
      browserAudio?.stop(); browserAudio = null;
    };
  }));

  function connectEvents() {
    if (!livePolling || ws?.readyState === WebSocket.OPEN || ws?.readyState === WebSocket.CONNECTING) return;
    try {
      ws = openEvents(handleEvent, (state) => {
        eventConnection = state;
        if ((state === 'closed' || state === 'error') && livePolling) {
          eventReconnectTimer ??= window.setTimeout(() => {
            eventReconnectTimer = null;
            ws = null;
            connectEvents();
          }, 1_000);
        }
      });
    } catch (error) {
      eventConnection = 'error';
      if (livePolling) eventReconnectTimer ??= window.setTimeout(() => { eventReconnectTimer = null; connectEvents(); }, 1_000);
    }
  }

  function scheduleLivePoll(delayMs: number) {
    if (!livePolling) return;
    if (livePollTimer) window.clearTimeout(livePollTimer);
    livePollTimer = window.setTimeout(() => {
      livePollTimer = null;
      void pollLiveData();
    }, delayMs);
  }

  async function pollLiveData() {
    if (!livePolling) return;
    if (livePollBusy) {
      scheduleLivePoll(250);
      return;
    }
    livePollBusy = true;
    try {
      nowMs = Date.now();
      if (spectrumConnection !== 'open' || spectrumStale) {
        if (spectrumConnection === 'open' && spectrumStale) {
          spectrumWs?.close();
          spectrumWs = null;
          reconnectSpectrum(0);
        }
        await pollSpectrum();
      }
      if (++livePollTick % 4 === 0) await pollRuntime();
    } finally {
      livePollBusy = false;
      scheduleLivePoll(document.visibilityState === 'visible' ? 250 : 1_000);
    }
  }

  function connectSpectrum() {
    if (!livePolling || spectrumWs?.readyState === WebSocket.OPEN || spectrumWs?.readyState === WebSocket.CONNECTING) return;
    spectrumWs = openSpectrum(
      (frame) => applyStreamSpectrum(frame),
      (state) => {
        spectrumConnection = state;
        if ((state === 'closed' || state === 'error') && livePolling) reconnectSpectrum(1_000);
      },
    );
  }

  function reconnectSpectrum(delayMs: number) {
    if (!livePolling || spectrumReconnectTimer) return;
    spectrumReconnectTimer = window.setTimeout(() => {
      spectrumReconnectTimer = null;
      connectSpectrum();
    }, delayMs);
  }

  function applyStreamSpectrum(frame: SpectrumStreamFrame) {
    if (lastSpectrumSequence > 0 && frame.sequence > lastSpectrumSequence + 1) {
      droppedSpectrumFrames += frame.sequence - lastSpectrumSequence - 1;
    }
    lastSpectrumSequence = frame.sequence;
    lastSpectrumAt = Date.now();
    nowMs = lastSpectrumAt;
    centerFreqHz = frame.centerFreqHz;
    sampleRateHz = frame.sampleRateHz;
    spectrumError = '';
    applySpectrum(frame.bins);
  }

  async function loadInitial() {
    try {
      const [bankList, status, storedSignals] = await Promise.all([Api.banks(), Api.deviceStatus(), Api.signalEvents(100)]);
      banks = bankList;
      deviceLabel = status.label;
      connected = status.connected;
      centerFreqHz = Number(status.center_freq_hz ?? 0);
      sampleRateHz = Number(status.sample_rate ?? 1);
      signalHistory = storedSignals;
      applyVfos(await Api.vfoStates());
      messages = await Api.decodedMessages(100);
    } catch (e) {
      console.warn('init failed', e);
      notice = `init failed: ${e}`;
    }
  }

  async function pollRuntime() {
    try {
      const [status, nextVfos] = await Promise.all([Api.deviceStatus(), Api.vfoStates()]);
      deviceLabel = status.label;
      connected = status.connected;
      centerFreqHz = Number(status.center_freq_hz ?? centerFreqHz);
      sampleRateHz = Number(status.sample_rate ?? sampleRateHz);
      applyVfos(nextVfos);
    } catch (e) { console.warn('runtime polling failed', e); }
  }

  function applySpectrum(bins: number[]) {
    spectrumBins = bins;
    if (drawPending) return;
    drawPending = true;
    requestAnimationFrame(() => {
      drawPending = false;
      drawSpectrum();
      drawWaterfall();
    });
  }

  async function pollSpectrum() {
    try {
      const spectrum = await Api.spectrum();
      // /spectrum is the reliable non-WS path; keep scanner state synchronized
      // here as well so a dropped event socket cannot leave dead VFO/UI chrome.
      activeRange = spectrum?.range ?? activeRange;
      scanRunning = Boolean(spectrum?.running);
      if (Array.isArray(spectrum?.bins) && spectrum.bins.length > 0) {
        spectrumError = '';
        lastSpectrumSequence = Number(spectrum.frame_sequence ?? lastSpectrumSequence);
        lastSpectrumAt = Date.now();
        applySpectrum(spectrum.bins);
      }
    } catch (e) {
      spectrumError = String(e);
    }
  }

  function handleEvent(ev: ScannerEvent) {
    switch (ev.kind) {
      case 'Spectrum':
        // Dedicated binary transport owns live FFT delivery. The event stream
        // remains a compatibility path for non-spectrum state.
        break;
      case 'VfoStates':
        applyVfos(ev.data);
        break;
      case 'DecodedMessage':
        messages = [ev.data, ...messages].slice(0, 200);
        break;
      case 'SignalHit':
        signalHistory = [ev.data, ...signalHistory].slice(0, 100);
        break;
    }
  }

  function ensureCanvasBacking(canvas: HTMLCanvasElement, width: number, height: number) {
    // Setting canvas width/height clears its bitmap. Keep backing dimensions
    // outside Svelte's reactive attributes and only initialize them once.
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
  }

  function bandGroup(bank: ScanRange): string {
    const name = bank.name.toLowerCase();
    if (name.includes('broadcast') || name.includes('fm')) return 'Broadcast';
    if (name.includes('sat') || name.includes('aircraft') || name.includes('ads-b')) return 'Satellite';
    if (name.includes('ism') || name.includes('sensor') || name.includes('lora')) return 'ISM';
    const midpoint = (bank.start_hz + bank.end_hz) / 2;
    if (midpoint < 30e6) return 'HF';
    if (midpoint < 300e6) return 'VHF';
    if (midpoint < 1e9) return 'UHF';
    if (midpoint < 6e9) return 'Microwave';
    return 'Other';
  }

  function setWaterfallGain(event: Event) {
    waterfallGain = Number((event.currentTarget as HTMLInputElement).value);
    localStorage.setItem('pulsescope.waterfall.gain', String(waterfallGain));
  }
  function setWaterfallPalette(event: Event) {
    waterfallPalette = (event.currentTarget as HTMLSelectElement).value === 'mono' ? 'mono' : 'classic';
    localStorage.setItem('pulsescope.waterfall.palette', waterfallPalette);
  }

  function drawSpectrum() {
    if (!canvas || spectrumBins.length === 0) return;
    ensureCanvasBacking(canvas, 900, 220);
    const ctx = canvas.getContext('2d')!;
    const w = canvas.width;
    const h = canvas.height;
    ctx.clearRect(0, 0, w, h);

    // Grid
    ctx.strokeStyle = '#1f2c36';
    ctx.lineWidth = 1;
    for (let i = 0; i <= 10; i++) {
      const x = (i / 10) * w;
      ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, h); ctx.stroke();
    }

    // Active VFO markers make tuning context visible even on a wide span.
    for (const vfo of vfos) {
      const normalized = (vfo.frequency_hz - (centerFreqHz - sampleRateHz / 2)) / sampleRateHz;
      if (normalized < 0 || normalized > 1) continue;
      const x = normalized * w;
      ctx.strokeStyle = vfo.muted ? '#64748b' : '#f59e0b';
      ctx.setLineDash([4, 3]); ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, h); ctx.stroke(); ctx.setLineDash([]);
    }
    ctx.strokeStyle = '#94a3b8'; ctx.setLineDash([2, 3]); ctx.beginPath(); ctx.moveTo(w / 2, 0); ctx.lineTo(w / 2, h); ctx.stroke(); ctx.setLineDash([]);

    // Spectrum trace. Use the immediate canvas path API: it works in both
    // Chromium's Tauri webview and normal browsers without Path2D cloning.
    const n = spectrumBins.length;
    const min = -100, max = 0;
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const x = (i / Math.max(1, n - 1)) * w;
      const norm = Math.max(0, Math.min(1, (spectrumBins[i] - min) / (max - min)));
      const y = h - norm * h;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = '#2dd4bf';
    ctx.lineWidth = 1.5;
    ctx.stroke();
  }

  function drawWaterfall() {
    if (!waterfallCanvas || spectrumBins.length === 0) return;
    ensureCanvasBacking(waterfallCanvas, 900, 180);
    if (typeof Worker !== 'undefined' && typeof waterfallCanvas.transferControlToOffscreen === 'function') {
      if (!waterfallWorker) {
        const offscreen = waterfallCanvas.transferControlToOffscreen();
        waterfallWorker = new Worker(new URL('../lib/waterfall-worker.ts', import.meta.url), { type: 'module' });
        waterfallWorker.postMessage({ canvas: offscreen, width: 900, height: 180 }, [offscreen]);
      }
      waterfallWorker.postMessage({ bins: spectrumBins, gain: waterfallGain, palette: waterfallPalette });
      return;
    }
    const ctx = waterfallCanvas.getContext('2d');
    if (!ctx) return;
    const w = waterfallCanvas.width;
    const h = waterfallCanvas.height;
    // Keep history outside the canvas. WebView render cycles may clear a
    // canvas bitmap; this typed buffer makes the waterfall deterministic.
    if (!waterfallPixels || waterfallPixels.length !== w * h * 4) {
      waterfallPixels = new Uint8ClampedArray(w * h * 4);
    }
    // Browser polling is deliberately low-rate on LAN/mobile. Write a small
    // stripe per accepted FFT frame so the waterfall visibly advances instead
    // of looking frozen after a single one-pixel row.
    const rowsPerFrame = 3;
    const rowBytes = w * 4;
    if (h > rowsPerFrame) waterfallPixels.copyWithin(rowBytes * rowsPerFrame, 0, rowBytes * (h - rowsPerFrame));
    for (let x = 0; x < w; x++) {
      const index = Math.min(spectrumBins.length - 1, Math.floor((x / w) * spectrumBins.length));
      const value = Math.max(0, Math.min(1, ((spectrumBins[index] + 100) / 80) * waterfallGain));
      const hue = 240 - value * 240;
      const c = 1 - Math.abs((hue / 60) % 2 - 1);
      const sector = Math.floor(hue / 60);
      const rgb = waterfallPalette === 'mono'
        ? [1, 1, 1]
        : sector === 0 ? [1, c, 0] : sector === 1 ? [c, 1, 0] : sector === 2 ? [0, 1, c] : sector === 3 ? [0, c, 1] : sector === 4 ? [c, 0, 1] : [1, 0, c];
      const pixel = x * 4;
      waterfallPixels[pixel] = Math.round(rgb[0] * value * 255);
      waterfallPixels[pixel + 1] = Math.round(rgb[1] * value * 255);
      waterfallPixels[pixel + 2] = Math.round(rgb[2] * value * 255);
      waterfallPixels[pixel + 3] = 255;
    }
    for (let row = 1; row < rowsPerFrame; row++) {
      waterfallPixels.set(waterfallPixels.subarray(0, rowBytes), row * rowBytes);
    }
    if (!waterfallImage || waterfallImage.width !== w || waterfallImage.height !== h) {
      waterfallImage = ctx.createImageData(w, h);
    }
    waterfallImage.data.set(waterfallPixels);
    ctx.putImageData(waterfallImage, 0, 0);
  }

  async function startScan(name: string) {
    activeRange = name;
    scanRunning = true;
    await Api.scanStart(name);
  }
  async function startQuickMode(match: string) {
    const range = banks.find((b) => b.name.toLowerCase().includes(match.toLowerCase()));
    if (range) await startScan(range.name);
  }

  async function stopScan() {
    scanRunning = false; activeRange = null;
    await Api.scanStop();
  }

  function frequencyAtSurface(target: HTMLCanvasElement, clientX: number): number {
    const rect = target.getBoundingClientRect();
    const fraction = Math.max(0, Math.min(1, (clientX - rect.left) / Math.max(1, rect.width)));
    return Math.max(1, Math.round(centerFreqHz - sampleRateHz / 2 + fraction * sampleRateHz));
  }

  function clearWaterfallHistory() {
    waterfallPixels = null;
    waterfallImage = null;
    if (waterfallWorker) waterfallWorker.postMessage({ clear: true });
  }

  async function applyCenterFrequency(frequencyHz: number) {
    if (!connected || !Number.isFinite(frequencyHz) || frequencyHz <= 0) return;
    if (centerTuneBusy) {
      pendingCenterHz = Math.round(frequencyHz);
      return;
    }
    const requestedHz = Math.round(frequencyHz);
    centerTuneBusy = true;
    pendingCenterHz = requestedHz;
    try {
      if (scanRunning) await stopScan();
      await Api.deviceFrequency(requestedHz);
      centerFreqHz = requestedHz;
      clearWaterfallHistory();
      notice = `Receiver centered on ${fmtHz(requestedHz)}`;
      window.setTimeout(() => { if (notice.startsWith('Receiver centered')) notice = ''; }, 1800);
    } catch (error) {
      notice = `Could not change center frequency: ${String(error)}`;
    } finally {
      centerTuneBusy = false;
      const queuedHz = pendingCenterHz;
      if (queuedHz !== null && queuedHz !== requestedHz) scheduleCenterFrequency(queuedHz, 0);
      else pendingCenterHz = null;
    }
  }

  function scheduleCenterFrequency(frequencyHz: number, delayMs = 180) {
    pendingCenterHz = Math.max(1, Math.round(frequencyHz));
    if (centerTuneTimer) window.clearTimeout(centerTuneTimer);
    centerTuneTimer = window.setTimeout(() => {
      centerTuneTimer = null;
      const requested = pendingCenterHz;
      if (requested !== null) void applyCenterFrequency(requested);
    }, delayMs);
  }

  function panCenter(fractionOfSpan: number) {
    if (sampleRateHz <= 0) return;
    scheduleCenterFrequency(displayedCenterHz + sampleRateHz * fractionOfSpan, 80);
  }

  function centerOnVfo() {
    const frequencyHz = vfos[0]?.frequency_hz;
    if (frequencyHz) scheduleCenterFrequency(frequencyHz, 0);
  }

  function setCenterFromInput(event: Event) {
    const mhz = Number((event.currentTarget as HTMLInputElement).value);
    if (Number.isFinite(mhz) && mhz > 0) scheduleCenterFrequency(mhz * 1e6, 0);
  }

  function panSurfaceWheel(event: WheelEvent) {
    if (!connected || sampleRateHz <= 0) return;
    event.preventDefault();
    const direction = Math.sign(Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY);
    const step = event.shiftKey ? 0.01 : 0.08;
    if (direction) scheduleCenterFrequency(displayedCenterHz + direction * sampleRateHz * step);
  }

  function beginSurfacePan(event: PointerEvent) {
    if (spectrumTool !== 'center' || !connected) return;
    const target = event.currentTarget as HTMLCanvasElement;
    panPointerId = event.pointerId;
    panStartX = event.clientX;
    panStartCenterHz = displayedCenterHz;
    panDragged = false;
    target.setPointerCapture(event.pointerId);
  }

  function moveSurfacePan(event: PointerEvent) {
    if (panPointerId !== event.pointerId || sampleRateHz <= 0) return;
    const target = event.currentTarget as HTMLCanvasElement;
    const distance = event.clientX - panStartX;
    if (Math.abs(distance) > 4) panDragged = true;
    pendingCenterHz = Math.max(1, Math.round(panStartCenterHz - distance / Math.max(1, target.clientWidth) * sampleRateHz));
  }

  function endSurfacePan(event: PointerEvent) {
    if (panPointerId !== event.pointerId) return;
    const target = event.currentTarget as HTMLCanvasElement;
    if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId);
    panPointerId = null;
    if (panDragged) {
      suppressSurfaceClick = true;
      const requested = pendingCenterHz;
      if (requested !== null) scheduleCenterFrequency(requested, 0);
      window.setTimeout(() => (suppressSurfaceClick = false), 0);
    }
  }

  async function tuneFromSpectrum(event: MouseEvent) {
    if (suppressSurfaceClick) return;
    const target = event.currentTarget as HTMLCanvasElement;
    if (sampleRateHz <= 0) return;
    const frequencyHz = frequencyAtSurface(target, event.clientX);
    if (spectrumTool === 'center') {
      scheduleCenterFrequency(frequencyHz, 0);
      return;
    }
    if (!vfos.length) return;
    await Api.vfoFrequency(vfos[0].id, frequencyHz);
    notice = `VFO ${vfos[0].id} tuned to ${fmtHz(frequencyHz)}`;
    window.setTimeout(() => { if (notice.startsWith('VFO ')) notice = ''; }, 1800);
  }

  async function tuneFromSpectrumKeyboard(event: KeyboardEvent) {
    if (!vfos.length || sampleRateHz <= 0) return;
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    const step = event.shiftKey ? 10_000 : 1_000;
    const direction = event.key === 'ArrowRight' ? 1 : -1;
    if (spectrumTool === 'center') {
      scheduleCenterFrequency(displayedCenterHz + direction * step, 0);
      return;
    }
    const frequencyHz = Math.max(1, Math.round(vfos[0].frequency_hz + direction * step));
    await Api.vfoFrequency(vfos[0].id, frequencyHz);
    notice = `VFO ${vfos[0].id} tuned to ${fmtHz(frequencyHz)}`;
  }

  async function setVfoFrequency(id: number, event: Event) {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    if (Number.isFinite(value) && value > 0) await Api.vfoFrequency(id, value);
  }
  async function setVfoMode(id: number, event: Event) {
    await Api.vfoMode(id, (event.currentTarget as HTMLSelectElement).value);
  }

  async function toggleVfoAudio(vfo: VfoState) {
    audioGesturePending = vfo.muted;
    if (vfo.muted) {
      try {
        await browserAudio?.start();
      } catch (error) {
        audioGesturePending = false;
        audioState = 'error';
        notice = `Browser audio could not start: ${String(error)}`;
        return;
      }
    }
    try {
      await Api.vfoMute(vfo.id, !vfo.muted);
    } finally {
      audioGesturePending = false;
    }
    if (!vfo.muted && vfos.filter((candidate) => !candidate.muted).length === 1) browserAudio?.stop();
  }

  async function identifyVfo(id: number) {
    try {
      const result = await Api.vfoIdentify(id);
      notice = result?.available === false ? result.reason : `VFO ${id} identification requested`;
    } catch (e) { notice = String(e); }
    setTimeout(() => (notice = ''), 3500);
  }

  function exportMessages() {
    const header = 'timestamp_ms,frequency_hz,protocol,message_type,address,content\\n';
    const csv = header + visibleMessages.map((m) => [m.timestamp_ms, m.frequency_hz, m.protocol, m.message_type, m.address, m.content]
      .map((value) => `"${String(value ?? '').replaceAll('"', '""')}"`).join(',')).join('\\n');
    const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv;charset=utf-8' }));
    const link = document.createElement('a');
    link.href = url;
    link.download = `pulsescope-messages-${Date.now()}.csv`;
    link.click();
    URL.revokeObjectURL(url);
  }

  function miniTrace(frequencyHz: number): string {
    if (spectrumBins.length < 2) return '';
    const centerBin = centerFreqHz > 0 ? Math.round(((frequencyHz - centerFreqHz) / sampleRateHz + 0.5) * spectrumBins.length) : Math.floor(spectrumBins.length / 2);
    const width = Math.max(16, Math.floor(spectrumBins.length / 10));
    const start = Math.max(0, Math.min(spectrumBins.length - width, centerBin - Math.floor(width / 2)));
    const points: string[] = [];
    for (let i = 0; i < 48; i++) {
      const index = Math.min(spectrumBins.length - 1, start + Math.floor((i / 47) * (width - 1)));
      const x = (i / 47) * 100;
      const norm = Math.max(0, Math.min(1, (spectrumBins[index] + 100) / 100));
      points.push(`${x.toFixed(1)},${(38 - norm * 32).toFixed(1)}`);
    }
    return points.join(' ');
  }

  function fmtHz(hz: number): string {
    if (hz >= 1e9) return (hz / 1e9).toFixed(3) + ' GHz';
    if (hz >= 1e6) return (hz / 1e6).toFixed(3) + ' MHz';
    if (hz >= 1e3) return (hz / 1e3).toFixed(0) + ' kHz';
    return hz + ' Hz';
  }
  function fmtTime(ms: number): string {
    return new Date(ms).toLocaleTimeString();
  }
</script>

<div class="scanner-layout">
  <!-- Scan-range sidebar -->
  <aside class="banks">
    <div class="banks-header">
      <h2>Scan Ranges</h2>
      <input bind:value={filter} placeholder="filter…" />
    </div>
    <div class="bank-groups">
      {#each groupedBanks as entry (entry.group)}
        <section class="bank-group">
          <h3>{entry.group}</h3>
          <ul>
            {#each entry.banks as b (b.name)}
              <li>
                <button
                  class="range-row"
                  class:active={activeRange === b.name}
                  onclick={() => (activeRange === b.name ? stopScan() : startScan(b.name))}
                >
                  <span class="range-name">{b.name}</span>
                  <span class="range-meta">
                    {fmtHz(b.start_hz)}–{fmtHz(b.end_hz)} · {b.mode.toUpperCase()}
                  </span>
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {:else}
        <div class="empty-banks">No supported scan ranges match this filter.</div>
      {/each}
    </div>
    {#if scanRunning}
      <button class="primary stop" onclick={stopScan}>■ Stop Scan</button>
    {/if}
  </aside>

  <!-- Main: spectrum + VFO tiles -->
  <section class="center">
    {#if notice}<div class="ui-notice" role="status">{notice}</div>{/if}
    <div class="command-strip card">
      <div class="quick-modes">
        <span class="strip-label">Listen or monitor</span>
        {#each quickModes as mode}
          <button class="quick" onclick={() => startQuickMode(mode.match)}>{mode.label}</button>
        {/each}
      </div>
      <div class="runtime-status">
        <span class="status-pill" class:on={connected}>● SDR {connected ? 'online' : 'offline'}</span>
        <span class="status-pill" class:on={scanRunning}>● Receiver {scanRunning ? 'active' : 'idle'}</span>
        <span class="status-pill" class:on={eventConnection === 'open'}>● Data {eventConnection === 'open' ? 'live' : 'reconnecting'}</span>
        <a href="#/settings" class="settings-link">Hardware</a>
      </div>
    </div>
    {#if !connected}
      <section class="setup-card card" role="status">
        <div>
          <strong>No SDR is connected</strong>
          <p>Connect a supported device to start the live spectrum and audio streams.</p>
        </div>
        <div class="setup-actions">
          <a class="primary" href="#/settings">Open device setup</a>
          <button onclick={() => void pollRuntime()}>Retry detection</button>
        </div>
      </section>
    {/if}
    <div class="device-strip card">
      <div>
        <span class="dot" class:on={connected}></span>
        {deviceLabel}
      </div>
      <div class="receiver-readout"><span>CENTER</span><strong>{displayedCenterHz > 0 ? fmtHz(displayedCenterHz) : 'Tuning…'}</strong><small>{sampleRateHz > 0 ? `${fmtHz(sampleRateHz)} visible span` : ''}</small></div>
      <div class="vfo-summary">{vfos.length} VFO{vfos.length === 1 ? '' : 's'} active</div>
      <div class="audio-status" class:on={audioState === 'playing'}>Audio: {audioState}</div>
    </div>

    <div class="spectrum-wrap card" class:stale={spectrumStale}>
      <div class="spectrum-heading">
        <h2>Spectrum <small class="fft-status">{spectrumError || (spectrumStale ? 'reconnecting…' : `${spectrumBins.length} bins · frame ${lastSpectrumSequence}${droppedSpectrumFrames ? ` · ${droppedSpectrumFrames} dropped` : ''}`)}</small></h2>
        <div class="spectrum-tools" role="group" aria-label="Spectrum interaction">
          <button class:active={spectrumTool === 'vfo'} onclick={() => (spectrumTool = 'vfo')}>Tune VFO</button>
          <button class:active={spectrumTool === 'center'} onclick={() => (spectrumTool = 'center')}>Move center</button>
        </div>
      </div>
      <div class="center-controls">
        <button onclick={() => panCenter(-0.25)} aria-label="Move center left by one quarter of the visible span">← ¼ span</button>
        <label><span>Center frequency</span><span class="frequency-entry"><input type="number" min="0.001" step="0.001" value={(displayedCenterHz / 1e6).toFixed(6)} onchange={setCenterFromInput} aria-label="Center frequency in megahertz" /><b>MHz</b></span></label>
        <button onclick={centerOnVfo} disabled={!vfos.length} title="Put VFO 0 in the middle of the visible spectrum">Center on VFO</button>
        <button onclick={() => panCenter(0.25)} aria-label="Move center right by one quarter of the visible span">¼ span →</button>
      </div>
      <p class="interaction-help">{spectrumTool === 'vfo' ? 'Click a signal to tune VFO 0. Scroll to move across the band.' : 'Click to make that frequency the new center, or drag the spectrum and waterfall left or right.'}</p>
      <canvas class:center-mode={spectrumTool === 'center'} bind:this={canvas} onclick={tuneFromSpectrum} onwheel={panSurfaceWheel} onpointerdown={beginSurfacePan} onpointermove={moveSurfacePan} onpointerup={endSurfacePan} onpointercancel={endSurfacePan} onkeydown={tuneFromSpectrumKeyboard} tabindex="0" role="slider" aria-valuemin="0" aria-valuemax={Math.max(1, sampleRateHz)} aria-valuenow={spectrumTool === 'center' ? displayedCenterHz : (vfos[0]?.frequency_hz ?? centerFreqHz)} aria-label={spectrumTool === 'center' ? 'Spectrum center control. Click or drag to change center frequency; use arrow keys for fine adjustment.' : 'Spectrum tuner. Click to tune VFO 0; scroll to pan center frequency; use arrow keys to fine tune.'} title={spectrumTool === 'center' ? 'Click or drag to move the receiver center' : 'Click to tune VFO 0; scroll to pan'}></canvas>
      {#if spectrumStale}<div class="stale-overlay">Spectrum reconnecting</div>{/if}
      <div class="waterfall-head"><h2 class="waterfall-title">Waterfall · live FFT history</h2><label>Gain <input aria-label="Waterfall gain" type="range" min="0.25" max="4" step="0.25" value={waterfallGain} oninput={setWaterfallGain} /></label><select aria-label="Waterfall palette" value={waterfallPalette} onchange={setWaterfallPalette}><option value="classic">Classic</option><option value="mono">Mono</option></select></div>
      <canvas class="waterfall" class:center-mode={spectrumTool === 'center'} bind:this={waterfallCanvas} onclick={tuneFromSpectrum} onwheel={panSurfaceWheel} onpointerdown={beginSurfacePan} onpointermove={moveSurfacePan} onpointerup={endSurfacePan} onpointercancel={endSurfacePan} aria-label="Live waterfall. Scroll to pan the receiver center; select Move center to click or drag."></canvas>
    </div>

    <section class="signal-history card">
      <div class="history-head"><h2>Persisted signal history</h2><button class="mini" onclick={async () => (signalHistory = await Api.signalEvents(100))}>Refresh</button></div>
      {#if signalHistory.length === 0}
        <div class="history-empty">No persisted signal hits</div>
      {:else}
        <div class="history-list">
          {#each signalHistory.slice(0, 12) as hit}
            <div class="history-row"><span>{hit.timestamp_ms ? fmtTime(hit.timestamp_ms) : 'live'}</span><b>{fmtHz(Number(hit.frequency_hz ?? 0))}</b><span>{hit.signal_class ?? hit.family ?? 'signal'}</span><span>SNR {Number(hit.snr_db ?? 0).toFixed(1)} dB</span></div>
          {/each}
        </div>
      {/if}
    </section>

    <div class="vfo-grid">
      {#each vfos as v (v.id)}
        <div class="vfo-tile card">
          <div class="vfo-freq">{fmtHz(v.frequency_hz)}</div>
          <div class="vfo-controls">
            <input class="freq-input" type="number" step="100" value={v.frequency_hz} aria-label="VFO {v.id} frequency" onchange={(e) => setVfoFrequency(v.id, e)} />
            <select aria-label="VFO {v.id} mode" value={v.mode} onchange={(e) => setVfoMode(v.id, e)}>
              <option value="nfm">NFM</option><option value="wfm">WFM</option><option value="am">AM</option><option value="lsb">LSB</option><option value="usb">USB</option>
            </select>
          </div>
          <div class="vfo-mode">{v.mode.toUpperCase()} · VFO {v.id}</div>
          <div class="vfo-signal-head">
            <span class="signal-dot" class:on={v.strength_db > -80}></span>
            <span>SNR {v.strength_db.toFixed(1)} dB</span>
          </div>
          <svg class="mini-spectrum" viewBox="0 0 100 40" preserveAspectRatio="none" aria-label="Live spectrum for VFO {v.id}">
            <polyline points={miniTrace(v.frequency_hz)} fill="none" stroke="var(--accent)" stroke-width="1.2" vector-effect="non-scaling-stroke" />
            <line x1="0" y1="38" x2="100" y2="38" stroke="var(--line-strong)" stroke-width="0.5" />
          </svg>
          <div class="vfo-bar">
            <label for="vfo-volume-{v.id}">Vol</label>
            <input
              id="vfo-volume-{v.id}"              type="range" min="0" max="1" step="0.01"
              value={v.volume}
              oninput={(e) => Api.vfoVolume(v.id, parseFloat((e.target as HTMLInputElement).value))}
            />
            <button class="listen" class:on={!v.muted} aria-label={v.muted ? `Listen to VFO ${v.id}` : `Mute VFO ${v.id}`}
              onclick={() => toggleVfoAudio(v)}>
              {v.muted ? '▶ Listen' : '■ Mute'}
            </button>
          </div>
          <div class="vfo-actions">
            <button class="mini" class:on={v.audio_agc} onclick={() => Api.vfoAgc(v.id, !v.audio_agc)}>AGC</button>
            <button class="mini" onclick={() => identifyVfo(v.id)}>ID</button>
          </div>
          <div class="vfo-strength">
            <div class="meter">
              <div class="meter-fill" style="width: {Math.max(0, Math.min(100, (v.strength_db + 120) / 1.2))}%"></div>
            </div>
            <span class="strength-val">{v.strength_db.toFixed(0)} dB</span>
          </div>
        </div>
      {/each}
    </div>
  </section>

  <!-- Message log -->
  <aside class="log card">
    <div class="dock-tabs">
      <button class="dock-tab" class:active={dockFilter === 'all'} onclick={() => (dockFilter = 'all')}>All <b>{messages.length}</b></button>
      <button class="dock-tab" class:active={dockFilter === 'trunk'} onclick={() => (dockFilter = 'trunk')}>Trunking</button>
      <button class="dock-tab" class:active={dockFilter === 'pag'} onclick={() => (dockFilter = 'pag')}>Paging</button>
      <button class="dock-tab" class:active={dockFilter === 'sensor'} onclick={() => (dockFilter = 'sensor')}>Sensors</button>
      <button class="dock-tab" class:active={dockFilter === 'air'} onclick={() => (dockFilter = 'air')}>Aircraft</button>
      <button class="dock-tab" class:active={dockFilter === 'ais'} onclick={() => (dockFilter = 'ais')}>AIS</button>
      <button class="dock-tab" class:active={dockFilter === 'aprs'} onclick={() => (dockFilter = 'aprs')}>APRS</button>
      <button class="dock-tab" class:active={dockFilter === 'lora'} onclick={() => (dockFilter = 'lora')}>LoRa</button>
    </div>
    <div class="dock-toolbar"><input bind:value={messageSearch} placeholder="Search decoded messages…" /><span class="dock-count">{visibleMessages.length} shown</span><button onclick={exportMessages} disabled={visibleMessages.length === 0}>Export CSV</button><button onclick={() => (messages = [])}>Clear</button></div>
    <div class="log-list">
      {#each visibleMessages as m (m.id ?? m.timestamp_ms + m.protocol)}
        <div class="log-row" data-proto={m.protocol}>
          <span class="log-ts">{fmtTime(m.timestamp_ms)}</span>
          <span class="log-proto">{m.protocol}</span>
          <span class="log-content">{m.content}</span>
        </div>
      {:else}
        <div class="log-empty">No messages yet</div>
      {/each}
    </div>
  </aside>
</div>

<style>
  .scanner-layout {
    display: grid;
    grid-template-columns: 260px 1fr;
    grid-template-rows: minmax(0, 1fr) 140px;
    gap: 8px;
    height: 100%;
    padding: 8px;
    overflow: hidden;
  }
  .banks {
    display: flex; flex-direction: column; gap: 8px;
    overflow: hidden;
    background: var(--bg-elev);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 8px;
  }
  .banks-header h2 { margin: 0 0 6px; font-size: 12px; color: var(--fg-dim); text-transform: uppercase; letter-spacing: 0.05em; }
  .bank-groups { overflow-y: auto; flex: 1; }
  .bank-group h3 { margin: 10px 4px 4px; color: var(--accent-2); font: 10px var(--mono); text-transform: uppercase; letter-spacing: .08em; }
  .bank-group ul { list-style: none; margin: 0; padding: 0; }
  .empty-banks { color: var(--fg-dim); font-size: 12px; padding: 12px 4px; }
  .banks li { margin: 2px 0; }
  .range-row {
    display: flex; flex-direction: column; width: 100%;
    background: transparent; border: 1px solid transparent;
    text-align: left; padding: 6px 8px; border-radius: 6px; cursor: pointer;
    color: var(--fg);
  }
  .range-row:hover { background: var(--bg-elev-2); }
  .range-row.active { background: var(--bg-elev-2); border-color: var(--accent); }
  .range-name { font-weight: 500; font-size: 13px; }
  .range-meta { font-size: 11px; color: var(--fg-dim); font-family: var(--mono); }
  .stop { margin-top: 8px; width: 100%; }

  .ui-notice { padding: 6px 10px; color: var(--warn); background: rgba(245,158,11,.12); border: 1px solid rgba(245,158,11,.35); border-radius: 4px; font-size: 12px; }
  .setup-card { display:flex; align-items:center; justify-content:space-between; gap:12px; border-color:rgba(245,158,11,.45); background:rgba(245,158,11,.08); }
  .setup-card strong { color:var(--warn); }
  .setup-card p { margin:3px 0 0; color:var(--fg-dim); font-size:12px; }
  .setup-actions { display:flex; align-items:center; gap:6px; flex-wrap:wrap; }
  .setup-actions a { display:inline-flex; align-items:center; min-height:30px; padding:6px 12px; border-radius:6px; text-decoration:none; }
  .command-strip { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 6px 8px; border-radius: 4px; background: #13294b; }
  .quick-modes, .runtime-status { display: flex; align-items: center; gap: 5px; flex-wrap: wrap; }
  .strip-label { color: var(--fg-dim); text-transform: uppercase; font: 10px var(--mono); margin-right: 4px; }
  .quick { padding: 4px 7px; font-size: 11px; border-radius: 3px; }
  .status-pill { color: var(--fg-dim); font: 10px var(--mono); }
  .status-pill.on { color: var(--ok); }
  .settings-link { color: var(--fg); text-decoration: none; font-size: 12px; border-left: 1px solid var(--line-strong); padding-left: 8px; }

  .center { display: flex; flex-direction: column; gap: 8px; overflow-y: auto; min-height: 0; padding-right: 2px; }
  .device-strip { display: flex; justify-content: space-between; align-items: center; gap: 16px; padding: 8px 12px; font-size: 13px; }
  .receiver-readout { display: flex; align-items: baseline; gap: 6px; font-family: var(--mono); }
  .receiver-readout span, .receiver-readout small { color: var(--fg-dim); font-size: 10px; }
  .receiver-readout strong { color: var(--accent); font-size: 14px; }
  .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: var(--danger); margin-right: 8px; }
  .dot.on { background: var(--ok); box-shadow: 0 0 6px var(--ok); }
  .vfo-summary { color: var(--fg-dim); }
  .audio-status { color: var(--fg-dim); text-transform: capitalize; }
  .audio-status.on { color: var(--ok); }

  .spectrum-wrap { flex: 0 0 auto; min-height: 250px; position: relative; }
  .spectrum-wrap.stale canvas { opacity: 0.45; }
  .stale-overlay { position:absolute; inset:36px 12px 112px; display:grid; place-items:center; color:var(--warn); background:rgb(7 12 18 / 55%); font:700 12px var(--mono); text-transform:uppercase; letter-spacing:.08em; pointer-events:none; }
  .spectrum-wrap canvas { display: block; width: 100%; height: 95px; background: var(--bg); border-radius: 4px; }
  .spectrum-wrap canvas { cursor:crosshair; }
  .spectrum-wrap canvas.center-mode { cursor:grab; touch-action:none; }
  .spectrum-wrap canvas.center-mode:active { cursor:grabbing; }
  .spectrum-heading { display:flex; align-items:center; justify-content:space-between; gap:10px; }
  .spectrum-heading h2 { margin:0; }
  .spectrum-tools { display:flex; gap:3px; padding:2px; border:1px solid var(--line); border-radius:6px; background:var(--bg); }
  .spectrum-tools button { min-height:28px; padding:4px 9px; border:0; font-size:11px; }
  .spectrum-tools button.active { color:#03120f; background:var(--accent); }
  .center-controls { display:grid; grid-template-columns:auto minmax(220px, 1fr) auto auto; align-items:end; gap:6px; margin:8px 0 2px; }
  .center-controls > button { min-height:36px; white-space:nowrap; }
  .center-controls label { display:flex; flex-direction:column; gap:3px; color:var(--fg-dim); font:10px var(--mono); text-transform:uppercase; letter-spacing:.05em; }
  .frequency-entry { display:flex; align-items:center; overflow:hidden; border:1px solid var(--line-strong); border-radius:6px; background:var(--bg); }
  .frequency-entry:focus-within { border-color:var(--accent); box-shadow:0 0 0 2px rgb(45 212 191 / 15%); }
  .frequency-entry input { width:100%; min-height:34px; padding:6px 9px; color:var(--accent); background:transparent; border:0; font:600 15px var(--mono); }
  .frequency-entry input:focus { outline:0; }
  .frequency-entry b { padding:0 9px; color:var(--fg-dim); font:11px var(--mono); }
  .interaction-help { margin:4px 0 7px; color:var(--fg-dim); font-size:11px; }
  .fft-status { color: var(--fg-dim); font: 10px var(--mono); font-weight: normal; }
  .waterfall-title { margin-top: 6px !important; }
  .waterfall-head { display:flex; align-items:center; gap:8px; }
  .waterfall-head h2 { flex:1; }
  .waterfall-head label, .waterfall-head select { font:10px var(--mono); color:var(--fg-dim); }
  .waterfall-head input { width:70px; vertical-align:middle; }
  .spectrum-wrap canvas.waterfall { height: 95px; image-rendering: pixelated; }

  .signal-history { order: 3; max-height: 190px; overflow: hidden; }
  .history-head { display: flex; justify-content: space-between; align-items: center; }
  .history-head h2 { margin-bottom: 0; }
  .history-list { overflow-y: auto; max-height: 145px; }
  .history-row { display: grid; grid-template-columns: 90px 110px 1fr 100px; gap: 8px; padding: 5px 0; border-top: 1px solid var(--line); color: var(--fg-dim); font: 10px var(--mono); }
  .history-row b { color: var(--accent); }
  .history-empty { color: var(--fg-dim); padding: 8px 0; font-size: 12px; }

  .vfo-grid { order: 2; display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 8px; }
  .vfo-tile { display: flex; flex-direction: column; gap: 4px; }
  .vfo-freq { font-family: var(--mono); font-size: 18px; font-weight: 600; color: var(--accent); }
  .vfo-mode { font-size: 11px; color: var(--fg-dim); text-transform: uppercase; letter-spacing: 0.05em; }
  .vfo-controls { display: flex; gap: 4px; }
  .vfo-actions { display: flex; gap: 3px; flex-wrap: wrap; }
  .vfo-signal-head { display: flex; align-items: center; gap: 5px; color: var(--fg-dim); font: 10px var(--mono); }
  .signal-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--danger); }
  .signal-dot.on { background: var(--ok); box-shadow: 0 0 5px var(--ok); }
  .mini-spectrum { width: 100%; height: 42px; background: #070c12; border: 1px solid var(--line); border-radius: 3px; }
  .freq-input { min-width: 0; width: 100%; font-family: var(--mono); font-size: 11px; }
  .vfo-controls select { font-size: 11px; }
  .vfo-bar { display: flex; align-items: center; gap: 6px; font-size: 11px; }
  .vfo-bar input[type=range] { flex: 1; }
  .vfo-bar .listen { min-width:74px; padding:5px 8px; color:var(--fg); border-color:var(--accent); }
  .vfo-bar .listen.on { color:#03120f; background:var(--accent); }
  .vfo-strength { display: flex; align-items: center; gap: 6px; }
  .meter { flex: 1; height: 6px; background: var(--bg); border-radius: 3px; overflow: hidden; }
  .meter-fill { height: 100%; background: linear-gradient(90deg, var(--ok), var(--warn), var(--danger)); transition: width 0.1s; }
  .strength-val { font-family: var(--mono); font-size: 10px; color: var(--fg-dim); width: 48px; text-align: right; }

  .log { grid-column: 1 / -1; min-height: 0; padding: 0; overflow: hidden; border-radius: 4px; }
  .dock-tabs { display: flex; align-items: center; gap: 3px; padding: 4px 8px; background: #13294b; border-bottom: 1px solid var(--line-strong); }
  .dock-tab { border: 0; background: transparent; color: var(--fg-dim); padding: 4px 9px; font-size: 11px; border-radius: 3px; }
  .dock-tab.active { color: var(--fg); background: var(--bg-elev-2); }
  .dock-tab b { color: var(--accent); font: 10px var(--mono); }
  .dock-toolbar { display: flex; gap: 4px; padding: 4px 8px; background: var(--bg-elev-2); }
  .dock-toolbar input { flex: 1; padding: 4px 7px; font-size: 11px; }
  .dock-count { color: var(--fg-dim); font: 10px var(--mono); padding: 0 6px; }
  .log-list { height: calc(100% - 66px); overflow-y: auto; font-family: var(--mono); font-size: 11px; }
  .log-row { padding: 4px 6px; border-bottom: 1px solid var(--line); display: grid; grid-template-columns: 56px 60px 1fr; gap: 6px; }
  .log-ts { color: var(--fg-dim); }
  .log-proto { color: var(--accent-2); text-transform: uppercase; font-size: 10px; }
  .log-content { color: var(--fg); word-break: break-word; white-space: pre-wrap; }
  .log-empty { color: var(--fg-dim); padding: 16px; text-align: center; }

  @media (max-width: 760px) {
    .scanner-layout { display:flex; flex-direction:column; height:auto; min-height:100%; padding:8px; gap:8px; overflow:visible; }
    .banks { max-height:none; order:3; padding:10px; }
    .banks-header { display:grid; grid-template-columns:1fr; gap:6px; }
    .banks-header input { min-height:42px; font-size:14px; }
    .bank-groups { max-height:280px; }
    .range-row { min-height:52px; padding:9px 10px; }
    .range-name { font-size:14px; }
    .center { order:1; overflow:visible; padding:0; gap:8px; }
    .command-strip { position:sticky; top:0; z-index:10; align-items:stretch; flex-direction:column; gap:8px; padding:10px; box-shadow:0 4px 14px rgba(0,0,0,.28); }
    .quick-modes { overflow-x:auto; flex-wrap:nowrap; scrollbar-width:none; }
    .quick-modes .strip-label { position:absolute; width:1px; height:1px; overflow:hidden; clip:rect(0 0 0 0); }
    .quick { flex:0 0 auto; min-height:40px; padding:8px 12px; }
    .runtime-status { justify-content:space-between; }
    .runtime-status .status-pill:nth-child(2), .runtime-status .status-pill:nth-child(3) { display:none; }
    .settings-link { margin-left:auto; }
    .setup-card { align-items:flex-start; flex-direction:column; }
    .device-strip { gap:8px; flex-wrap:wrap; padding:11px; }
    .device-strip > div:first-child { width:100%; font-size:14px; }
    .vfo-summary { margin-left:auto; }
    .audio-status { width:100%; padding-top:6px; border-top:1px solid var(--line); }
    .receiver-readout { order:-1; width:100%; justify-content:space-between; }
    .spectrum-wrap { min-height:0; }
    .spectrum-wrap h2 { display:flex; flex-wrap:wrap; gap:5px; align-items:baseline; }
    .spectrum-heading { align-items:flex-start; }
    .spectrum-heading h2 { flex:1; }
    .spectrum-tools button { min-height:40px; padding:7px 10px; }
    .center-controls { grid-template-columns:1fr 1fr; }
    .center-controls label { grid-column:1 / -1; grid-row:1; }
    .center-controls > button { min-height:44px; }
    .center-controls > button:nth-of-type(2) { grid-column:1 / -1; grid-row:3; }
    .frequency-entry input { min-height:42px; font-size:18px; }
    .interaction-help { font-size:12px; line-height:1.4; }
    .spectrum-wrap canvas { height:150px; }
    .spectrum-wrap canvas.waterfall { height:120px; }
    .waterfall-head { flex-wrap:wrap; }
    .waterfall-head h2 { min-width:180px; }
    .vfo-grid { grid-template-columns:1fr; gap:10px; }
    .vfo-tile { padding:12px; gap:8px; }
    .vfo-freq { font-size:22px; }
    .vfo-bar .listen { min-width:110px; min-height:44px; font-size:14px; }
    .signal-history { display:none; }
    .log { display:none; }
  }

  /* Short laptop / WebView windows: live RF controls beat an empty log dock. */
  @media (max-height: 850px) {
    .scanner-layout { grid-template-rows: minmax(0, 1fr); }
    .log { display: none; }
    .spectrum-wrap { min-height: 230px; }
    .spectrum-wrap canvas, .spectrum-wrap canvas.waterfall { height: 80px; }
    .vfo-tile { padding: 8px; }
    .signal-history { display: none; }
  }
</style>
