<script module lang="ts">
  import { Api as LiveApi } from '$lib/api';

  let liveHubStarted = false;
  let liveHubBusy = false;
  let liveHubTick = 0;
  /** A singleton browser polling hub. It survives an accidental duplicate
   * Svelte mount; views only subscribe/unsubscribe from its events. */
  function ensureLiveHub() {
    if (liveHubStarted || typeof window === 'undefined') return;
    liveHubStarted = true;
    const poll = async () => {
      if (liveHubBusy) return;
      liveHubBusy = true;
      try {
        const spectrum = await LiveApi.spectrum();
        window.dispatchEvent(new CustomEvent('pulsescope:spectrum', { detail: spectrum }));
        if (++liveHubTick % 4 === 0) {
          const [status, vfos] = await Promise.all([LiveApi.deviceStatus(), LiveApi.vfoStates()]);
          window.dispatchEvent(new CustomEvent('pulsescope:runtime', { detail: { status, vfos } }));
        }
      } catch (error) {
        window.dispatchEvent(new CustomEvent('pulsescope:poll-error', { detail: String(error) }));
      } finally { liveHubBusy = false; }
    };
    void poll();
    // requestAnimationFrame remains active for a foreground phone/browser
    // dashboard, unlike timer intervals which Chromium can freeze after a
    // handful of callbacks when the page loses scheduler priority.
    let lastPollAt = 0;
    const frame = (now: number) => {
      if (now - lastPollAt >= 250) { lastPollAt = now; void poll(); }
      window.requestAnimationFrame(frame);
    };
    window.requestAnimationFrame(frame);
  }
</script>

<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { Api, openEvents, type ScanRange, type VfoState, type DecodedMessage, type ScannerEvent } from '$lib/api';
  import { browser } from '$app/environment';

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
  let waterfallPixels: Uint8ClampedArray | null = null;
  let waterfallGain = $state(1);
  let waterfallPalette = $state('classic');
  let initialLoadInFlight = false;

  const filteredBanks = $derived(
    banks.filter((b) => b.name.toLowerCase().includes(filter.toLowerCase()))
  );

  const visibleMessages = $derived(
    messages.filter((m) => {
      const matchesTab = dockFilter === 'all' || m.protocol.toLowerCase().includes(dockFilter);
      const needle = messageSearch.trim().toLowerCase();
      return matchesTab && (!needle || `${m.protocol} ${m.content}`.toLowerCase().includes(needle));
    })
  );

  const quickModes = [
    { label: 'ADS-B 1090', match: 'ADS-B' },
    { label: 'AIS 162', match: 'AIS' },
    { label: 'ACARS 130', match: 'ACARS' },
    { label: 'APRS 144', match: 'APRS' },
    { label: '433 Sensors', match: 'ISM 433' },
    { label: '915 Sensors', match: 'ISM 915' },
    { label: 'Pagers', match: 'Pagers' }
  ];


  onMount(() => {
    if (!browser) return;
    waterfallGain = Math.max(0.25, Math.min(4, Number(localStorage.getItem('pulsescope.waterfall.gain') ?? 1)) || 1);
    waterfallPalette = localStorage.getItem('pulsescope.waterfall.palette') === 'mono' ? 'mono' : 'classic';
    const onSpectrum = (event: Event) => {
      const spectrum = (event as CustomEvent).detail;
      activeRange = spectrum?.range ?? activeRange;
      scanRunning = Boolean(spectrum?.running);
      if (Array.isArray(spectrum?.bins) && spectrum.bins.length) {
        spectrumError = '';
        void applySpectrum(spectrum.bins);
      }
    };
    const onRuntime = (event: Event) => {
      const { status, vfos: nextVfos } = (event as CustomEvent).detail;
      deviceLabel = status.label; connected = status.connected;
      centerFreqHz = Number(status.center_freq_hz ?? centerFreqHz);
      sampleRateHz = Number(status.sample_rate ?? sampleRateHz); vfos = nextVfos;
    };
    const onPollError = (event: Event) => { spectrumError = (event as CustomEvent).detail; };
    window.addEventListener('pulsescope:spectrum', onSpectrum);
    window.addEventListener('pulsescope:runtime', onRuntime);
    window.addEventListener('pulsescope:poll-error', onPollError);
    ensureLiveHub();
    void (async () => {
      await loadInitial();
      try { ws = openEvents(handleEvent); }
      catch (e) { console.warn('event ws unavailable; singleton polling remains active', e); }
    })();
    return () => {
      window.removeEventListener('pulsescope:spectrum', onSpectrum);
      window.removeEventListener('pulsescope:runtime', onRuntime);
      window.removeEventListener('pulsescope:poll-error', onPollError);
      ws?.close(); ws = null;
    };
  });

  // Even when the route is mounted by the hash router without a
  // visible navigation event, Svelte 5 runes still fire `$effect`
  // on the client. Use it as a belt-and-suspenders to ensure the
  // initial data load always runs at least once in the browser.
  $effect(() => {
    if (!browser) return;
    // The static-router hydration path reliably runs this effect. Seed the
    // first FFT frame here; the interval below maintains it afterwards.
    void pollSpectrum();
    if (banks.length === 0 && !notice.startsWith('init')) {
      notice = 'init…';
      loadInitial().finally(() => { notice = notice === 'init…' ? '' : notice; });
    }
  });

  async function loadInitial() {
    if (initialLoadInFlight || banks.length > 0) return;
    initialLoadInFlight = true;
    let lastError: unknown;
    // Tauri creates the webview before its setup hook's local API listener is
    // necessarily bound. Retry the short startup race instead of leaving the
    // dashboard permanently stuck at "Failed to fetch".
    for (let attempt = 0; attempt < 8; attempt++) {
      try {
        const [bankList, status, storedSignals] = await Promise.all([Api.banks(), Api.deviceStatus(), Api.signalEvents(100)]);
        banks = bankList;
        deviceLabel = status.label;
        connected = status.connected;
        centerFreqHz = Number(status.center_freq_hz ?? 0);
        sampleRateHz = Number(status.sample_rate ?? 1);
        signalHistory = storedSignals;
        vfos = await Api.vfoStates();
        messages = await Api.decodedMessages(100);
        notice = '';
        initialLoadInFlight = false;
        return;
      } catch (e) {
        lastError = e;
        await new Promise((resolve) => window.setTimeout(resolve, 150 * (attempt + 1)));
      }
    }
    initialLoadInFlight = false;
    console.warn('init failed', lastError);
    notice = `init failed: ${lastError}`;
  }

  async function pollRuntime() {
    try {
      const [status, nextVfos] = await Promise.all([Api.deviceStatus(), Api.vfoStates()]);
      deviceLabel = status.label;
      connected = status.connected;
      centerFreqHz = Number(status.center_freq_hz ?? centerFreqHz);
      sampleRateHz = Number(status.sample_rate ?? sampleRateHz);
      vfos = nextVfos;
    } catch (e) { console.warn('runtime polling failed', e); }
  }

  async function applySpectrum(bins: number[]) {
    spectrumBins = bins;
    await tick();
    drawSpectrum();
    drawWaterfall();
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
        await applySpectrum(spectrum.bins);
      }
    } catch (e) {
      spectrumError = String(e);
    }
  }

  function handleEvent(ev: ScannerEvent) {
    switch (ev.kind) {
      case 'Spectrum':
        void applySpectrum(ev.data.bins);
        break;
      case 'VfoStates':
        vfos = ev.data;
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
    const image = ctx.createImageData(w, h);
    image.data.set(waterfallPixels);
    ctx.putImageData(image, 0, 0);
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

  async function tuneFromSpectrum(event: MouseEvent) {
    const target = event.currentTarget as HTMLCanvasElement;
    if (!vfos.length || sampleRateHz <= 0) return;
    const rect = target.getBoundingClientRect();
    const fraction = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
    const frequencyHz = Math.round(centerFreqHz - sampleRateHz / 2 + fraction * sampleRateHz);
    await Api.vfoFrequency(vfos[0].id, frequencyHz);
    notice = `VFO ${vfos[0].id} tuned to ${fmtHz(frequencyHz)}`;
    window.setTimeout(() => { if (notice.startsWith('VFO ')) notice = ''; }, 1800);
  }

  async function setVfoFrequency(id: number, event: Event) {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    if (Number.isFinite(value) && value > 0) await Api.vfoFrequency(id, value);
  }
  async function setVfoMode(id: number, event: Event) {
    await Api.vfoMode(id, (event.currentTarget as HTMLSelectElement).value);
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

  function unsupportedAction(action: string) {
    notice = `${action} is not implemented yet`;
    setTimeout(() => (notice = ''), 3500);
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
    <ul>
      {#each filteredBanks as b (b.name)}
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
    {#if scanRunning}
      <button class="primary stop" onclick={stopScan}>■ Stop Scan</button>
    {/if}
  </aside>

  <!-- Main: spectrum + VFO tiles -->
  <section class="center">
    {#if notice}<div class="ui-notice" role="status">{notice}</div>{/if}
    <div class="command-strip card">
      <div class="quick-modes">
        <span class="strip-label">Quick Modes</span>
        {#each quickModes as mode}
          <button class="quick" onclick={() => startQuickMode(mode.match)}>{mode.label}</button>
        {/each}
      </div>
      <div class="runtime-status">
        <span class="status-pill" class:on={connected}>● {connected ? 'PWR' : 'OFF'}</span>
        <span class="status-pill" class:on={scanRunning}>● {scanRunning ? 'SCANNING' : 'IDLE'}</span>
        <a href="#/settings" class="settings-link">⚙ Settings</a>
      </div>
    </div>
    <div class="device-strip card">
      <div>
        <span class="dot" class:on={connected}></span>
        {deviceLabel}
      </div>
      <div class="receiver-readout"><span>RECEIVER</span><strong>{centerFreqHz > 0 ? fmtHz(centerFreqHz) : 'Tuning…'}</strong><small>{sampleRateHz > 0 ? `${fmtHz(sampleRateHz)} span` : ''}</small></div>
      <div class="vfo-summary">{vfos.length} VFO{vfos.length === 1 ? '' : 's'} active</div>
    </div>

    <div class="spectrum-wrap card">
      <h2>Spectrum <small class="fft-status">{spectrumError || (spectrumBins.length ? `${spectrumBins.length} FFT bins` : 'waiting for FFT')}</small></h2>
      <canvas bind:this={canvas} onclick={tuneFromSpectrum} title="Click to tune VFO 0"></canvas>
      <div class="waterfall-head"><h2 class="waterfall-title">Waterfall · live FFT history</h2><label>Gain <input aria-label="Waterfall gain" type="range" min="0.25" max="4" step="0.25" value={waterfallGain} oninput={setWaterfallGain} /></label><select aria-label="Waterfall palette" value={waterfallPalette} onchange={setWaterfallPalette}><option value="classic">Classic</option><option value="mono">Mono</option></select></div>
      <canvas class="waterfall" bind:this={waterfallCanvas} aria-label="Live waterfall from FFT bins"></canvas>
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
            <button class="mini" class:off={v.muted}
              onclick={() => Api.vfoMute(v.id, !v.muted)}>
              {v.muted ? '🔇' : '🔊'}
            </button>
          </div>
          <div class="vfo-actions">
            <button class="mini" class:on={v.audio_agc} onclick={() => Api.vfoAgc(v.id, !v.audio_agc)}>AGC</button>
            <button class="mini" onclick={() => identifyVfo(v.id)}>ID</button>
            <button class="mini" onclick={() => unsupportedAction('Hold')}>Hold</button>
            <button class="mini" onclick={() => unsupportedAction('Per-VFO recording')}>REC</button>
            <button class="mini" onclick={() => unsupportedAction('VFO zoom')}>Zoom</button>
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
  .banks ul { list-style: none; margin: 0; padding: 0; overflow-y: auto; flex: 1; }
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

  .spectrum-wrap { flex: 0 0 auto; min-height: 250px; }
  .spectrum-wrap canvas { display: block; width: 100%; height: 95px; background: var(--bg); border-radius: 4px; }
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
  .vfo-bar button.mini { padding: 2px 6px; font-size: 11px; }
  .vfo-bar button.mini.off { opacity: 0.5; }
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
    .scanner-layout { display:flex; flex-direction:column; height:auto; min-height:100%; padding:6px; gap:6px; overflow:visible; }
    .banks { max-height:220px; order:2; }
    .center { order:1; overflow:visible; padding:0; }
    .command-strip { align-items:flex-start; flex-direction:column; gap:6px; }
    .device-strip { gap:6px; flex-wrap:wrap; padding:7px; }
    .receiver-readout { order:-1; width:100%; justify-content:space-between; }
    .spectrum-wrap { min-height:0; }
    .spectrum-wrap canvas, .spectrum-wrap canvas.waterfall { height:110px; }
    .waterfall-head { flex-wrap:wrap; }
    .waterfall-head h2 { min-width:180px; }
    .vfo-grid { grid-template-columns:1fr; }
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
