<script lang="ts">
  import { untrack } from 'svelte';
  import { Api, openEvents, openSpectrum, type ScanRange, type VfoState, type DecodedMessage, type ScannerEvent, type SpectrumStreamFrame, type ReceiverBookmark } from '$lib/api';
  import { BrowserAudio, type BrowserAudioState } from '$lib/browser-audio';
  import { WaterfallCanvas } from '$lib/waterfall-canvas';
  import {
    autoDisplayLevels,
    formatSpectrumFrequency,
    gainStageLabel,
    horizontalDbGridLines,
    isCommonRfSetting,
    loadSpectrumDisplayConfig,
    normalizeDb,
    PeakHoldTrace,
    saveSpectrumDisplayConfig,
    sampleBinLinear,
    SpectrumSmoother,
    type SpectrumDisplayConfig,
  } from '$lib/spectrum-display';

  let banks: ScanRange[] = $state([]);
  let bookmarks: ReceiverBookmark[] = $state([]);
  let bookmarkLabel = $state('');
  let activeRange: string | null = $state(null);
  let vfos: VfoState[] = $state([]);
  let messages: DecodedMessage[] = $state([]);
  let signalHistory: any[] = $state([]);
  let deviceCaps: any = $state(null);
  let spectrumBins: number[] = $state([]);
  let spectrumError = $state('');
  let deviceLabel = $state('—');
  let connected = $state(false);
  let scanRunning = $state(false);
  let centerFreqHz = $state(0);
  let sampleRateHz = $state(1);
  let appliedBandwidthHz = $state(0);
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
  const waterfallCanvasRenderer = new WaterfallCanvas();
  const spectrumSmoother = new SpectrumSmoother();
  const peakHold = new PeakHoldTrace();
  let renderFps = $state(0);
  let renderFrames = 0;
  let renderWindowStarted = performance.now();
  let drawPending = false;
  let waterfallGain = $state(1);
  let displayConfig = $state<SpectrumDisplayConfig>(loadSpectrumDisplayConfig());
  let rfPanelOpen = $state(true);
  let rfControlBusy = $state(false);
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
  let listenerSessionId = '';
  let listenerSessionRevision = 0;
  let banksCollapsed = $state(false);
  let logExpanded = $state(false);
  let showShortcuts = $state(false);
  let dismissedOnboarding = $state(false);
  let isCompactLayout = $state(false);

  const spectrumStale = $derived(lastSpectrumAt === 0 || nowMs - lastSpectrumAt > 2_000);
  const spectrumLive = $derived(spectrumConnection === 'open' && !spectrumStale && spectrumBins.length > 0);
  const spectrumStatusLabel = $derived(
    spectrumError
      ? 'error'
      : spectrumLive
        ? 'live'
        : spectrumBins.length > 0
          ? 'polling'
          : connected
            ? 'waiting'
            : 'offline',
  );
  const displayedCenterHz = $derived(pendingCenterHz ?? centerFreqHz);
  const activeBank = $derived(banks.find((bank) => bank.name === activeRange));
  const scanProgress = $derived(activeBank && activeBank.end_hz > activeBank.start_hz
    ? Math.max(0, Math.min(100, ((centerFreqHz - activeBank.start_hz) / (activeBank.end_hz - activeBank.start_hz)) * 100))
    : 0);
  const foundSignals = $derived.by(() => {
    const seen = new Set<string>();
    return signalHistory.filter((hit) => {
      const bandwidth = Math.max(1, Number(hit.bandwidth_hz ?? activeBank?.channel_bw_hz ?? 12_500));
      const key = `${hit.range_name ?? ''}:${Math.round(Number(hit.frequency_hz ?? 0) / bandwidth)}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    }).slice(0, 10);
  });

  let noiseFloorDb = $state(-120);
  let voiceNoiseReject = $state(true);
  let showAllBands = $state(false);
  let scanWorkspaceOpen = $state(true);
  let scanLocked = $state(false);
  let scanHolding = $state(false);

  const VOICE_STARTER_BANDS = [
    'FM Broadcast',
    'NOAA Weather',
    '2m Amateur',
    'Aircraft AM',
    'Marine VHF',
    '70cm Amateur',
  ];

  const filteredBanks = $derived(
    banks.filter((b) => b.name.toLowerCase().includes(filter.toLowerCase())),
  );

  const featuredVoiceBanks = $derived(
    VOICE_STARTER_BANDS
      .map((name) => banks.find((bank) => bank.name === name))
      .filter((bank): bank is ScanRange => Boolean(bank)),
  );

  const enabledVoiceBanks = $derived(banks.filter((bank) => bank.enabled));
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
  const allVfosMuted = $derived(vfos.length > 0 && vfos.every((vfo) => vfo.muted));
  const showGettingStarted = $derived(
    connected
      && !dismissedOnboarding
      && !scanRunning
      && !activeRange
      && (spectrumBins.length === 0 || allVfosMuted),
  );
  const audibleVfo = $derived(vfos.find((vfo) => !vfo.muted) ?? vfos[0] ?? null);
  const deviceDriver = $derived(deviceCaps?.identity?.driver ?? deviceCaps?.driver ?? '');
  const commonRfSettings = $derived((deviceCaps?.settings ?? []).filter((setting: any) => isCommonRfSetting(setting)));
  const expertRfSettings = $derived((deviceCaps?.settings ?? []).filter((setting: any) => !isCommonRfSetting(setting)));

  function applyVfos(nextVfos: VfoState[]) {
    vfos = nextVfos;
    const audible = nextVfos.find((vfo) => !vfo.muted) ?? nextVfos[0];
    if (audible) browserAudio?.setMetadata(`${(audible.frequency_hz / 1e6).toFixed(5)} MHz · ${audible.mode.toUpperCase()}`, activeRange ?? 'PulseScope receiver');
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
    displayConfig = loadSpectrumDisplayConfig();
    waterfallGain = Math.max(0.25, Math.min(4, Number(localStorage.getItem('pulsescope.waterfall.gain') ?? 1)) || 1);
    voiceNoiseReject = localStorage.getItem('pulsescope.voice-noise-reject') !== '0';
    scanWorkspaceOpen = localStorage.getItem('pulsescope.ui.scan-workspace') !== '0';
    spectrumSmoother.setAlpha(displayConfig.smoothing);
    banksCollapsed = localStorage.getItem('pulsescope.ui.banksCollapsed') === '1';
    logExpanded = localStorage.getItem('pulsescope.ui.logExpanded') === '1';
    dismissedOnboarding = localStorage.getItem('pulsescope.ui.onboarding.dismissed') === '1';
    const compactQuery = window.matchMedia('(max-width: 760px)');
    const updateCompact = () => { isCompactLayout = compactQuery.matches; };
    updateCompact();
    compactQuery.addEventListener('change', updateCompact);
    listenerSessionId = localStorage.getItem('pulsescope.listener.id')
      || globalThis.crypto?.randomUUID?.()
      || `listener-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    localStorage.setItem('pulsescope.listener.id', listenerSessionId);
    const onSpectrum = (event: Event) => {
      const spectrum = (event as CustomEvent).detail;
      activeRange = spectrum?.range ?? activeRange;
      scanRunning = Boolean(spectrum?.running);
      scanLocked = Boolean(spectrum?.locked);
      scanHolding = Boolean(spectrum?.holding);
      if (Array.isArray(spectrum?.bins) && spectrum.bins.length) {
        spectrumError = '';
        applySpectrum(spectrum.bins);
      }
    };
    const onRuntime = (event: Event) => {
      const { status, vfos: nextVfos } = (event as CustomEvent).detail;
      deviceLabel = status.label; connected = status.connected;
      centerFreqHz = Number(status.center_freq_hz ?? centerFreqHz);
      sampleRateHz = Number(status.sample_rate ?? sampleRateHz);
      appliedBandwidthHz = Number(status.bandwidth_hz ?? appliedBandwidthHz); applyVfos(nextVfos);
    };
    const onPollError = (event: Event) => { spectrumError = (event as CustomEvent).detail; };
    window.addEventListener('pulsescope:spectrum', onSpectrum);
    window.addEventListener('pulsescope:runtime', onRuntime);
    window.addEventListener('pulsescope:poll-error', onPollError);
    livePolling = true;
    scheduleLivePoll(0);
    // A WebSocket is an acceleration path, not a prerequisite for the
    // receiver. HTTP polling must remain alive when a mobile browser, captive
    // portal, or privacy setting rejects a socket during page startup.
    connectSpectrum();
    const onVisibility = () => {
      if (document.visibilityState === 'visible') scheduleLivePoll(0);
    };
    document.addEventListener('visibilitychange', onVisibility);
    window.addEventListener('keydown', handleGlobalKeydown);
    void (async () => {
      await loadInitial();
      connectEvents();
    })();
    return () => {
      window.removeEventListener('pulsescope:spectrum', onSpectrum);
      window.removeEventListener('pulsescope:runtime', onRuntime);
      window.removeEventListener('pulsescope:poll-error', onPollError);
      document.removeEventListener('visibilitychange', onVisibility);
      window.removeEventListener('keydown', handleGlobalKeydown);
      compactQuery.removeEventListener('change', updateCompact);
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
      waterfallCanvasRenderer.detach();
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
    try {
      spectrumWs = openSpectrum(
        (frame) => applyStreamSpectrum(frame),
        (state) => {
          spectrumConnection = state;
          if ((state === 'closed' || state === 'error') && livePolling) reconnectSpectrum(1_000);
        },
      );
    } catch (error) {
      spectrumConnection = 'error';
      spectrumError = `Spectrum socket unavailable: ${error instanceof Error ? error.message : String(error)}`;
      reconnectSpectrum(1_000);
    }
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
    // Device status is the minimum viable receiver UI. Load it independently
    // so a nonessential history/bookmark request cannot leave the entire page
    // looking offline on a working radio.
    try {
      const status = await Api.deviceStatus();
      deviceLabel = status.label;
      connected = status.connected;
      centerFreqHz = Number(status.center_freq_hz ?? 0);
      sampleRateHz = Number(status.sample_rate ?? 1);
      appliedBandwidthHz = Number(status.bandwidth_hz ?? 0);
    } catch (e) {
      console.warn('device status failed', e);
      notice = `Receiver status unavailable: ${e instanceof Error ? e.message : String(e)}`;
    }
    const [banksResult, signalsResult, capabilitiesResult, bookmarksResult, vfosResult, messagesResult, listenerResult] = await Promise.allSettled([
      Api.banks(), Api.signalEvents(100), Api.deviceCapabilities(), Api.bookmarksV2(),
      Api.vfoStates(), Api.decodedMessages(100), syncListenerView(),
    ]);
    if (banksResult.status === 'fulfilled') banks = banksResult.value;
    if (signalsResult.status === 'fulfilled') signalHistory = signalsResult.value;
    if (capabilitiesResult.status === 'fulfilled') deviceCaps = capabilitiesResult.value;
    if (bookmarksResult.status === 'fulfilled') bookmarks = bookmarksResult.value.bookmarks;
    if (vfosResult.status === 'fulfilled') applyVfos(vfosResult.value);
    if (messagesResult.status === 'fulfilled') messages = messagesResult.value;
    if (listenerResult.status === 'rejected') {
      console.warn('listener session initialization failed', listenerResult.reason);
    }
  }

  async function pollRuntime() {
    try {
      const [status, nextVfos] = await Promise.all([Api.deviceStatus(), Api.vfoStates()]);
      deviceLabel = status.label;
      connected = status.connected;
      centerFreqHz = Number(status.center_freq_hz ?? centerFreqHz);
      sampleRateHz = Number(status.sample_rate ?? sampleRateHz);
      appliedBandwidthHz = Number(status.bandwidth_hz ?? appliedBandwidthHz);
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
      renderFrames += 1;
      const renderedAt = performance.now();
      if (renderedAt - renderWindowStarted >= 1_000) {
        renderFps = Math.round(renderFrames * 1_000 / (renderedAt - renderWindowStarted));
        renderFrames = 0;
        renderWindowStarted = renderedAt;
      }
    });
  }

  async function pollSpectrum() {
    try {
      const spectrum = await Api.spectrum();
      // /spectrum is the reliable non-WS path; keep scanner state synchronized
      // here as well so a dropped event socket cannot leave dead VFO/UI chrome.
      activeRange = spectrum?.range ?? activeRange;
      scanRunning = Boolean(spectrum?.running);
      scanLocked = Boolean(spectrum?.locked);
      scanHolding = Boolean(spectrum?.holding);
      if (Array.isArray(spectrum?.bins) && spectrum.bins.length > 0) {
        spectrumError = '';
        lastSpectrumSequence = Number(spectrum.frame_sequence ?? lastSpectrumSequence);
        lastSpectrumAt = Date.now();
        noiseFloorDb = Number(spectrum?.noise_floor_db ?? noiseFloorDb);
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

  function canvasBackingSize(canvas: HTMLCanvasElement, fallbackWidth: number, fallbackHeight: number) {
    const rect = canvas.getBoundingClientRect();
    const scale = Math.max(1, Math.min(2, window.devicePixelRatio || 1));
    return {
      width: Math.max(320, Math.round((rect.width || fallbackWidth) * scale)),
      height: Math.max(80, Math.round((rect.height || fallbackHeight) * scale)),
    };
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

  function persistDisplayConfig() {
    saveSpectrumDisplayConfig(displayConfig);
    spectrumSmoother.setAlpha(displayConfig.smoothing);
    spectrumSmoother.reset();
    peakHold.reset();
    if (spectrumBins.length) {
      drawSpectrum();
      drawWaterfall();
    }
  }

  function setWaterfallGain(event: Event) {
    waterfallGain = Number((event.currentTarget as HTMLInputElement).value);
    localStorage.setItem('pulsescope.waterfall.gain', String(waterfallGain));
    if (spectrumBins.length) drawWaterfall();
  }

  function setDisplayPalette(event: Event) {
    const value = (event.currentTarget as HTMLSelectElement).value;
    displayConfig.palette = value === 'mono' || value === 'classic' || value === 'openwebrx' ? value : 'openwebrx';
    persistDisplayConfig();
  }

  function setDisplaySmoothing(event: Event) {
    displayConfig.smoothing = Number((event.currentTarget as HTMLInputElement).value);
    persistDisplayConfig();
  }

  function setDisplayMinDb(event: Event) {
    displayConfig.minDb = Number((event.currentTarget as HTMLInputElement).value);
    if (displayConfig.maxDb <= displayConfig.minDb + 10) displayConfig.maxDb = displayConfig.minDb + 50;
    persistDisplayConfig();
  }

  function setDisplayMaxDb(event: Event) {
    displayConfig.maxDb = Number((event.currentTarget as HTMLInputElement).value);
    if (displayConfig.maxDb <= displayConfig.minDb + 10) displayConfig.minDb = displayConfig.maxDb - 50;
    persistDisplayConfig();
  }

  function autoSpectrumLevels() {
    if (!spectrumBins.length) return;
    const levels = autoDisplayLevels(spectrumBins);
    displayConfig.minDb = levels.minDb;
    displayConfig.maxDb = levels.maxDb;
    persistDisplayConfig();
  }

  function toggleRfPanel() {
    rfPanelOpen = !rfPanelOpen;
    localStorage.setItem('pulsescope.ui.rf-panel', rfPanelOpen ? '1' : '0');
  }

  function scrollToSection(id: string) {
    const node = document.getElementById(id);
    if (!node) return;
    node.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  function openBookmarksSection() {
    banksCollapsed = false;
    localStorage.setItem('pulsescope.ui.banksCollapsed', '0');
    scrollToSection('band-presets');
  }

  function openRfSection() {
    rfPanelOpen = true;
    localStorage.setItem('pulsescope.ui.rf-panel', '1');
    scrollToSection('rf-controls');
  }

  function openEventsSection() {
    logExpanded = true;
    localStorage.setItem('pulsescope.ui.logExpanded', '1');
    scrollToSection('events-log');
  }

  async function refreshDeviceCaps() {
    try {
      deviceCaps = await Api.deviceCapabilities();
    } catch (e) {
      notice = `RF controls unavailable: ${e instanceof Error ? e.message : String(e)}`;
    }
  }

  async function setDeviceControl(control: string, value: string | number | boolean) {
    if (rfControlBusy) return;
    rfControlBusy = true;
    try {
      const result = await Api.deviceControl(control, value);
      deviceCaps = result.capabilities ?? result;
    } catch (e) {
      notice = String(e);
    } finally {
      rfControlBusy = false;
    }
  }

  function displayBins(): number[] {
    if (!spectrumBins.length) return spectrumBins;
    if (displayConfig.smoothing <= 0) return spectrumBins;
    spectrumSmoother.setAlpha(displayConfig.smoothing);
    return spectrumSmoother.process(spectrumBins);
  }

  function drawSpectrum() {
    if (!canvas || spectrumBins.length === 0) return;
    const backing = canvasBackingSize(canvas, 1200, 200);
    ensureCanvasBacking(canvas, backing.width, backing.height);
    const ctx = canvas.getContext('2d')!;
    const w = canvas.width;
    const h = canvas.height;
    const minDb = displayConfig.minDb;
    const maxDb = displayConfig.maxDb;
    const dpr = window.devicePixelRatio || 1;
    const bins = displayBins();
    const peaks = displayConfig.peakHold ? peakHold.process(bins) : null;
    ctx.clearRect(0, 0, w, h);

    const labelPad = Math.round(18 * dpr);
    const plotTop = labelPad;
    const plotHeight = h - labelPad;

    ctx.strokeStyle = '#1a242d';
    ctx.lineWidth = 1;
    for (const db of horizontalDbGridLines(minDb, maxDb, 5)) {
      const norm = normalizeDb(db, minDb, maxDb);
      const y = plotTop + plotHeight - norm * plotHeight;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
      ctx.fillStyle = '#64748b';
      ctx.font = `${Math.max(9, Math.round(9 * dpr))}px ui-monospace, monospace`;
      ctx.textAlign = 'left';
      ctx.fillText(`${Math.round(db)} dB`, 4 * dpr, Math.max(plotTop + 10 * dpr, y - 2 * dpr));
    }

    for (let i = 0; i <= 10; i++) {
      const x = (i / 10) * w;
      ctx.strokeStyle = '#1f2c36';
      ctx.beginPath();
      ctx.moveTo(x, plotTop);
      ctx.lineTo(x, h);
      ctx.stroke();
      const frequency = centerFreqHz - sampleRateHz / 2 + (i / 10) * sampleRateHz;
      ctx.fillStyle = '#94a3b8';
      ctx.font = `${Math.max(10, Math.round(10 * dpr))}px ui-monospace, monospace`;
      ctx.textAlign = i === 0 ? 'left' : i === 10 ? 'right' : 'center';
      ctx.fillText(formatSpectrumFrequency(frequency), x, 12 * dpr);
    }

    for (const vfo of vfos) {
      const normalized = (vfo.frequency_hz - (centerFreqHz - sampleRateHz / 2)) / sampleRateHz;
      if (normalized < 0 || normalized > 1) continue;
      const x = normalized * w;
      const passbandHz = vfo.mode === 'wfm' ? 200_000 : vfo.mode === 'nfm' ? 12_500 : vfo.mode === 'am' ? 10_000 : 3_000;
      const passbandWidth = Math.max(2, passbandHz / sampleRateHz * w);
      ctx.fillStyle = vfo.muted ? 'rgba(100,116,139,.10)' : 'rgba(45,212,191,.14)';
      ctx.fillRect(x - passbandWidth / 2, plotTop, passbandWidth, plotHeight);
      ctx.strokeStyle = vfo.muted ? '#64748b' : '#f59e0b';
      ctx.setLineDash([4, 3]);
      ctx.beginPath();
      ctx.moveTo(x, plotTop);
      ctx.lineTo(x, h);
      ctx.stroke();
      ctx.setLineDash([]);
    }
    ctx.strokeStyle = '#94a3b8';
    ctx.setLineDash([2, 3]);
    ctx.beginPath();
    ctx.moveTo(w / 2, plotTop);
    ctx.lineTo(w / 2, h);
    ctx.stroke();
    ctx.setLineDash([]);

    const traceY = (db: number) => plotTop + plotHeight - normalizeDb(db, minDb, maxDb) * plotHeight;

    ctx.beginPath();
    for (let x = 0; x < w; x += 1) {
      const db = sampleBinLinear(bins, x / Math.max(1, w - 1));
      const y = traceY(db);
      if (x === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.lineTo(w, h);
    ctx.lineTo(0, h);
    ctx.closePath();
    const fill = ctx.createLinearGradient(0, plotTop, 0, h);
    fill.addColorStop(0, 'rgba(45,212,191,0.35)');
    fill.addColorStop(1, 'rgba(45,212,191,0.02)');
    ctx.fillStyle = fill;
    ctx.fill();

    ctx.beginPath();
    for (let x = 0; x < w; x += 1) {
      const db = sampleBinLinear(bins, x / Math.max(1, w - 1));
      const y = traceY(db);
      if (x === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = '#2dd4bf';
    ctx.lineWidth = Math.max(1, 1.5 * dpr);
    ctx.stroke();

    if (peaks) {
      ctx.beginPath();
      for (let x = 0; x < w; x += 1) {
        const db = sampleBinLinear(peaks, x / Math.max(1, w - 1));
        const y = traceY(db);
        if (x === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.strokeStyle = 'rgba(251,191,36,0.85)';
      ctx.lineWidth = Math.max(1, 1 * dpr);
      ctx.stroke();
    }
  }

  function drawWaterfall() {
    if (!waterfallCanvas || spectrumBins.length === 0) return;
    waterfallCanvasRenderer.draw(spectrumBins, {
      gain: waterfallGain,
      palette: displayConfig.palette,
      minDb: displayConfig.minDb,
      maxDb: displayConfig.maxDb,
      rowsPerFrame: 1,
    });
  }

  function bindWaterfallCanvas(node: HTMLCanvasElement) {
    waterfallCanvas = node;
    waterfallCanvasRenderer.attach(node);
    if (spectrumBins.length) drawWaterfall();
    return {
      destroy() {
        waterfallCanvasRenderer.detach();
      },
    };
  }

  function bindSpectrumCanvas(node: HTMLCanvasElement) {
    canvas = node;
    if (spectrumBins.length) drawSpectrum();
    return {
      destroy() {
        canvas = undefined as unknown as HTMLCanvasElement;
      },
    };
  }

  async function startScan(name: string) {
    activeRange = name;
    scanRunning = true;
    await Api.scanStart(name);
  }

  async function startVoiceScan(bank: ScanRange) {
    if (activeRange === bank.name && scanRunning) {
      await stopScan();
      return;
    }
    if (voiceNoiseReject && bank.squelch_db < 16) {
      try {
        const result = await Api.updateChannelBank(bank.name, { squelch_db: 18 });
        const updated = result.bank ?? { ...bank, squelch_db: 18 };
        const index = banks.findIndex((item) => item.name === bank.name);
        if (index >= 0) banks[index] = { ...banks[index], ...updated };
      } catch (e) {
        notice = `Could not raise squelch: ${e instanceof Error ? e.message : String(e)}`;
      }
    }
    await startScan(bank.name);
    notice = `Scanning ${bank.name} · squelch ${banks.find((b) => b.name === bank.name)?.squelch_db?.toFixed(0) ?? '—'} dB above noise`;
    window.setTimeout(() => { if (notice.startsWith('Scanning ')) notice = ''; }, 2500);
  }

  async function setScanSquelch(event: Event) {
    if (!activeBank) return;
    const value = Number((event.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(value)) return;
    const index = banks.findIndex((bank) => bank.name === activeBank.name);
    if (index >= 0) banks[index].squelch_db = value;
    try {
      const result = await Api.updateChannelBank(activeBank.name, { squelch_db: value });
      if (result.bank && index >= 0) banks[index] = result.bank;
    } catch (e) {
      notice = String(e);
    }
  }

  function toggleVoiceNoiseReject(event: Event) {
    voiceNoiseReject = (event.currentTarget as HTMLInputElement).checked;
    localStorage.setItem('pulsescope.voice-noise-reject', voiceNoiseReject ? '1' : '0');
  }

  function toggleScanWorkspace() {
    scanWorkspaceOpen = !scanWorkspaceOpen;
    localStorage.setItem('pulsescope.ui.scan-workspace', scanWorkspaceOpen ? '1' : '0');
  }

  function openScanSection() {
    scanWorkspaceOpen = true;
    localStorage.setItem('pulsescope.ui.scan-workspace', '1');
    scrollToSection('scan-workspace');
  }

  async function startQuickMode(match: string) {
    const range = banks.find((b) => b.name.toLowerCase().includes(match.toLowerCase()));
    if (range) await startVoiceScan(range);
  }

  async function stopScan() {
    scanRunning = false; activeRange = null;
    scanLocked = false;
    scanHolding = false;
    await Api.scanStop();
  }

  async function startEnabledBanks() {
    if (!enabledVoiceBanks.length) {
      notice = 'Enable at least one bank in Settings, then scan enabled banks.';
      return;
    }
    activeRange = enabledVoiceBanks[0].name;
    scanRunning = true;
    try {
      await Api.scanStart('enabled');
      notice = `Scanning ${enabledVoiceBanks.length} enabled banks`;
      window.setTimeout(() => { if (notice.startsWith('Scanning ')) notice = ''; }, 2500);
    } catch (error) {
      scanRunning = false;
      notice = `Could not scan enabled banks: ${error instanceof Error ? error.message : String(error)}`;
    }
  }

  async function startBookmarkScan() {
    const enabledBookmarks = bookmarks.filter((bookmark) => bookmark.enabled !== false);
    if (!enabledBookmarks.length) {
      notice = 'Save at least one frequency before scanning bookmarks.';
      return;
    }
    activeRange = 'Bookmarks';
    scanRunning = true;
    try {
      await Api.scanStart('Bookmarks');
      notice = `Scanning ${enabledBookmarks.length} saved frequencies`;
      window.setTimeout(() => { if (notice.startsWith('Scanning ')) notice = ''; }, 2500);
    } catch (error) {
      scanRunning = false;
      notice = `Could not scan bookmarks: ${error instanceof Error ? error.message : String(error)}`;
    }
  }

  async function skipCurrentHit() {
    try {
      await Api.scanSkip();
      scanLocked = false;
      notice = 'Skipped this frequency for the rest of the survey';
      window.setTimeout(() => { if (notice.startsWith('Skipped ')) notice = ''; }, 2000);
    } catch (error) {
      notice = `Could not skip: ${error instanceof Error ? error.message : String(error)}`;
    }
  }

  async function lockoutCurrentHit() {
    try {
      await Api.scanLockout();
      scanLocked = false;
      notice = 'Locked out this frequency until you remove it from the blacklist';
      window.setTimeout(() => { if (notice.startsWith('Locked out ')) notice = ''; }, 2500);
    } catch (error) {
      notice = `Could not lock out: ${error instanceof Error ? error.message : String(error)}`;
    }
  }

  async function toggleScanHold() {
    try {
      if (scanLocked) {
        await Api.scanUnlock();
        scanLocked = false;
        notice = 'Scan resumed';
      } else {
        await Api.scanLock();
        scanLocked = true;
        notice = 'Holding this channel until you resume';
      }
      window.setTimeout(() => { if (notice.startsWith('Scan resumed') || notice.startsWith('Holding ')) notice = ''; }, 2000);
    } catch (error) {
      notice = `Could not change hold: ${error instanceof Error ? error.message : String(error)}`;
    }
  }

  function frequencyAtSurface(target: HTMLCanvasElement, clientX: number): number {
    const rect = target.getBoundingClientRect();
    const fraction = Math.max(0, Math.min(1, (clientX - rect.left) / Math.max(1, rect.width)));
    return Math.max(1, Math.round(centerFreqHz - sampleRateHz / 2 + fraction * sampleRateHz));
  }

  function availableSampleRates() {
    const preferred = [2_000_000, 5_000_000, 8_000_000, 10_000_000];
    const ranges = deviceCaps?.sample_rate_ranges_hz ?? [];
    return preferred.filter((rate) => ranges.some((range: any) => rate >= range.minimum && rate <= range.maximum));
  }

  async function setVisibleSpan(event: Event) {
    const sampleRate = Number((event.currentTarget as HTMLSelectElement).value);
    if (!sampleRate || sampleRate === sampleRateHz) return;
    try {
      const result = await Api.deviceSampleRate(sampleRate);
      sampleRateHz = Number(result.status?.sample_rate ?? sampleRate);
      appliedBandwidthHz = Number(result.bandwidth_hz ?? result.status?.bandwidth_hz ?? 0);
      deviceCaps = result.capabilities ?? deviceCaps;
      clearWaterfallHistory();
      notice = `Capture set to ${(sampleRateHz / 1e6).toFixed(0)} MSPS with ${(appliedBandwidthHz / 1e6).toFixed(3)} MHz RF bandwidth`;
      await syncListenerView();
    } catch (error) { notice = `Could not change visible spectrum: ${String(error)}`; }
  }

  async function tuneFoundSignal(hit: any) {
    if (!vfos.length) return;
    const frequencyHz = Number(hit.frequency_hz);
    audioGesturePending = true;
    try {
      await browserAudio?.start();
      // A survey result may be anywhere in a much wider band than the current
      // capture window. Make this explicit action deterministic: move the RF
      // window first, then park the listening VFO at its centre.
      await Api.deviceFrequency(frequencyHz);
      centerFreqHz = frequencyHz;
      clearWaterfallHistory();
      await Api.vfoFrequency(vfos[0].id, frequencyHz);
      await Api.vfoMute(vfos[0].id, false);
      notice = `Capture window centered and listening at ${fmtHz(frequencyHz)}`;
      await syncListenerView();
    } catch (error) {
      browserAudio?.stop();
      notice = `Could not listen to signal: ${String(error)}`;
    } finally { audioGesturePending = false; }
  }

  async function tuneBookmark(bookmark: ReceiverBookmark) {
    await tuneFoundSignal({ frequency_hz: bookmark.frequency_hz });
    if (vfos[0] && bookmark.mode) await Api.vfoMode(vfos[0].id, bookmark.mode);
  }

  async function saveCurrentBookmark() {
    const vfo = vfos[0];
    const label = bookmarkLabel.trim();
    if (!vfo || !label) return;
    try {
      await Api.saveBookmarkV2({ label, frequency_hz: vfo.frequency_hz, mode: vfo.mode, bandwidth_hz: activeBank?.channel_bw_hz ?? 12_500, enabled: true });
      bookmarks = (await Api.bookmarksV2()).bookmarks;
      bookmarkLabel = '';
      notice = `Saved ${label} at ${fmtHz(vfo.frequency_hz)}`;
    } catch (error) { notice = `Could not save bookmark: ${String(error)}`; }
  }

  async function removeBookmark(bookmark: ReceiverBookmark) {
    if (bookmark.id === undefined) return;
    await Api.deleteBookmarkV2(bookmark.id);
    bookmarks = bookmarks.filter((item) => item.id !== bookmark.id);
  }

  function clearWaterfallHistory() {
    waterfallCanvasRenderer.clear();
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
      await Api.deviceFrequency(requestedHz);
      centerFreqHz = requestedHz;
      clearWaterfallHistory();
      notice = `Receiver centered on ${fmtHz(requestedHz)}`;
      await syncListenerView();
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

  async function syncListenerView(retry = true) {
    if (!listenerSessionId || !centerFreqHz || !sampleRateHz) return;
    try {
      const result = await Api.saveListenerSessionV2({
        id: listenerSessionId,
        client_name: /Mobi|Android/i.test(navigator.userAgent) ? 'Mobile browser' : 'Desktop browser',
        receiver_id: 'receiver-0',
        view_center_hz: Math.round(centerFreqHz),
        view_span_hz: Math.round(Math.min(sampleRateHz, appliedBandwidthHz || sampleRateHz)),
        active_vfo_id: vfos[0]?.id ?? null,
        expected_revision: listenerSessionRevision,
      });
      listenerSessionRevision = Number(result.session?.revision ?? listenerSessionRevision);
    } catch (error: any) {
      if (retry && error?.status === 409) {
        const result: any = await Api.listenerSessionsV2();
        listenerSessionRevision = Number(result.sessions?.find((session: any) => session.id === listenerSessionId)?.revision ?? 0);
        await syncListenerView(false);
      }
    }
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
      // A survey may have moved the hardware window away from this parked
      // VFO. Listening is an explicit takeover: recenter first, then unmute.
      if (vfo.muted) await Api.vfoFrequency(vfo.id, vfo.frequency_hz);
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
      const norm = Math.max(0, Math.min(1, normalizeDb(spectrumBins[index], displayConfig.minDb, displayConfig.maxDb)));
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

  function toggleBanksCollapsed() {
    banksCollapsed = !banksCollapsed;
    localStorage.setItem('pulsescope.ui.banksCollapsed', banksCollapsed ? '1' : '0');
  }

  function toggleLogExpanded() {
    logExpanded = !logExpanded;
    localStorage.setItem('pulsescope.ui.logExpanded', logExpanded ? '1' : '0');
  }

  function dismissOnboarding() {
    dismissedOnboarding = true;
    localStorage.setItem('pulsescope.ui.onboarding.dismissed', '1');
  }

  function handleGlobalKeydown(event: KeyboardEvent) {
    const target = event.target as HTMLElement | null;
    const tag = target?.tagName;
    if (tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA' || target?.isContentEditable) return;
    if (event.key === '?' || (event.shiftKey && event.key === '/')) {
      event.preventDefault();
      showShortcuts = !showShortcuts;
      return;
    }
    if (event.key === 'Escape' && showShortcuts) {
      showShortcuts = false;
      return;
    }
    if (event.key === ' ' && vfos.length) {
      event.preventDefault();
      void toggleVfoAudio(audibleVfo!);
      return;
    }
    if ((event.key === 'b' || event.key === 'B') && !event.metaKey && !event.ctrlKey) {
      toggleBanksCollapsed();
      return;
    }
    if (!scanRunning || event.metaKey || event.ctrlKey) return;
    if (event.key === 's' || event.key === 'S') {
      event.preventDefault();
      void skipCurrentHit();
      return;
    }
    if (event.key === 'l' || event.key === 'L') {
      event.preventDefault();
      void lockoutCurrentHit();
      return;
    }
    if (event.key === 'h' || event.key === 'H') {
      event.preventDefault();
      void toggleScanHold();
    }
  }
</script>

<div class="scanner-layout" class:compact={isCompactLayout} class:banks-collapsed={banksCollapsed} class:log-expanded={logExpanded}>
  <!-- Bookmarks sidebar -->
  <aside id="band-presets" class="banks bookmarks-aside" class:collapsed={banksCollapsed}>
    <div class="banks-rail">
      <button
        class="panel-toggle banks-toggle"
        type="button"
        aria-expanded={!banksCollapsed}
        aria-label={banksCollapsed ? 'Show bookmarks' : 'Hide bookmarks'}
        onclick={toggleBanksCollapsed}
      >
        {banksCollapsed ? '▸' : '▾'} Saved
      </button>
    </div>
    {#if !banksCollapsed}
    <section class="bookmark-section bookmark-primary">
      <h2>Saved frequencies</h2>
      <p class="banks-help">Quick return to stations you marked while listening.</p>
      <div class="bookmark-add"><input bind:value={bookmarkLabel} placeholder="Name current VFO" aria-label="Bookmark name" onkeydown={(event) => { if (event.key === 'Enter') void saveCurrentBookmark(); }} /><button onclick={saveCurrentBookmark} disabled={!bookmarkLabel.trim() || !vfos.length}>Save</button></div>
      <ul class="bookmark-list">
        {#each bookmarks as bookmark (bookmark.id)}
          <li><button class="bookmark-tune" onclick={() => tuneBookmark(bookmark)}><span>{bookmark.label}</span><small>{fmtHz(bookmark.frequency_hz)} · {bookmark.mode.toUpperCase()}</small></button><button class="bookmark-delete" aria-label={`Delete ${bookmark.label}`} onclick={() => removeBookmark(bookmark)}>×</button></li>
        {:else}<li class="empty-banks">No saved frequencies yet.</li>{/each}
      </ul>
    </section>
    {/if}
  </aside>

  <!-- Main: spectrum + VFO tiles -->
  <section class="center">
    {#if notice}<div class="ui-notice" role="status">{notice}</div>{/if}

    <section id="scan-workspace" class="scan-workspace card section-card" class:collapsed={!scanWorkspaceOpen}>
      <header class="section-header">
        <div>
          <h2>Voice scan</h2>
          <p class="section-lead">Pick a band to search, or scan enabled banks and saved frequencies. Delay resumes after squelch closes; Hold stays on the hit. Skip lockouts the current frequency.</p>
        </div>
        <button type="button" class="mini section-toggle" onclick={toggleScanWorkspace} aria-expanded={scanWorkspaceOpen}>
          {scanWorkspaceOpen ? 'Collapse' : 'Expand'}
        </button>
      </header>
      {#if scanWorkspaceOpen}
        <div class="starter-bands" role="list" aria-label="Popular voice bands">
          {#each featuredVoiceBanks as bank (bank.name)}
            <button
              type="button"
              class="band-chip"
              class:active={activeRange === bank.name && scanRunning}
              onclick={() => startVoiceScan(bank)}
            >
              <strong>{bank.name}</strong>
              <small>{fmtHz(bank.start_hz)}–{fmtHz(bank.end_hz)} · {bank.mode.toUpperCase()}</small>
            </button>
          {:else}
            <p class="empty-banks">Band list still loading…</p>
          {/each}
        </div>
        <div class="scan-toolbar">
          <label class="toggle-field voice-reject">
            <input type="checkbox" checked={voiceNoiseReject} onchange={toggleVoiceNoiseReject} />
            <span>Reject RF noise <small>Raises squelch and gates on voice audio while listening</small></span>
          </label>
          {#if scanRunning}
            <div class="scan-live-controls">
              {#if activeBank}
              <label class="squelch-field">
                <span>Squelch <b>{activeBank.squelch_db.toFixed(0)} dB</b> above noise</span>
                <input
                  type="range"
                  min="6"
                  max="30"
                  step="1"
                  value={activeBank.squelch_db}
                  aria-label="Scan squelch in dB above noise floor"
                  oninput={setScanSquelch}
                  onchange={setScanSquelch}
                />
              </label>
              {/if}
              <span class="noise-readout">Noise floor {noiseFloorDb.toFixed(0)} dBFS</span>
              <div class="scan-actions">
                <button type="button" onclick={skipCurrentHit}>Skip</button>
                <button type="button" onclick={lockoutCurrentHit}>Lockout</button>
                <button type="button" class:active={scanLocked} onclick={toggleScanHold}>{scanLocked ? 'Resume' : 'Hold'}</button>
                <button type="button" class="primary stop" onclick={stopScan}>■ Stop scan</button>
              </div>
            </div>
          {:else}
            <div class="scan-search">
              <input bind:value={filter} placeholder="Search all bands…" aria-label="Search band presets" />
              <button type="button" class:primary={showAllBands} onclick={() => (showAllBands = !showAllBands)}>
                {showAllBands ? 'Hide band list' : 'Browse all bands'}
              </button>
              <button type="button" onclick={startEnabledBanks} disabled={!enabledVoiceBanks.length}>
                Scan enabled ({enabledVoiceBanks.length})
              </button>
              <button type="button" onclick={startBookmarkScan} disabled={!bookmarks.length}>
                Scan bookmarks
              </button>
            </div>
          {/if}
        </div>
        {#if showAllBands && !scanRunning}
          <div class="all-bands-panel">
            {#each groupedBanks as entry (entry.group)}
              <section class="bank-group">
                <h3>{entry.group}</h3>
                <div class="starter-bands compact">
                  {#each entry.banks as bank (bank.name)}
                    <button type="button" class="band-chip" onclick={() => startVoiceScan(bank)}>
                      <strong>{bank.name}</strong>
                      <small>{fmtHz(bank.start_hz)}–{fmtHz(bank.end_hz)}</small>
                    </button>
                  {/each}
                </div>
              </section>
            {:else}
              <p class="empty-banks">No bands match “{filter}”.</p>
            {/each}
          </div>
        {/if}
      {/if}
    </section>

    <div class="status-strip card">
      <div class="runtime-status">
        <span class="status-pill" class:on={connected}>● Radio {connected ? 'online' : 'offline'}</span>
        <span class="status-pill" class:on={spectrumLive}>● Spectrum {spectrumStatusLabel}</span>
        <span class="status-pill" class:on={scanRunning}>● Scan {scanRunning ? (scanLocked ? 'HOLD' : scanHolding ? 'delay' : activeRange ?? 'running') : 'idle'}</span>
        <span class="status-pill" class:on={eventConnection === 'open'}>● Events {eventConnection === 'open' ? 'live' : 'reconnecting'}</span>
        {#if !isCompactLayout}
          <button class="layout-toggle" type="button" aria-pressed={logExpanded} onclick={toggleLogExpanded}>
            {logExpanded ? 'Hide events' : 'Show events'}
          </button>
          <button class="layout-toggle" type="button" aria-pressed={showShortcuts} onclick={() => (showShortcuts = !showShortcuts)}>Shortcuts</button>
        {/if}
        <a href="#/settings" class="settings-link">Device setup</a>
      </div>
    </div>
    {#if isCompactLayout}
      <nav class="mobile-workspace-bar card" aria-label="Workspace sections">
        <button type="button" class:active={scanWorkspaceOpen} onclick={openScanSection}>Scan</button>
        <button type="button" class:active={rfPanelOpen} onclick={openRfSection} disabled={!connected}>RF</button>
        <button type="button" class:active={logExpanded} onclick={openEventsSection}>Events</button>
        <button type="button" class:active={showShortcuts} onclick={() => (showShortcuts = !showShortcuts)}>Help</button>
        <button type="button" onclick={() => scrollToSection('spectrum-display')}>Spectrum</button>
      </nav>
    {/if}
    {#if showShortcuts}
      <section class="shortcuts-card card" aria-label="Shortcuts and gestures">
        <div class="shortcuts-head">
          <strong>{isCompactLayout ? 'Touch and keyboard help' : 'Keyboard shortcuts'}</strong>
          <button type="button" class="mini" onclick={() => (showShortcuts = false)}>Close</button>
        </div>
        {#if isCompactLayout}
          <ul>
            <li>Tap spectrum or waterfall to tune · scroll to pan the hardware window</li>
            <li>Drag on spectrum with <b>Pan window</b> selected to move center without moving the VFO</li>
            <li>Use <b>RF</b> for gain, AGC, antenna, and Bias-T while listening</li>
            <li><b>Bands</b> and <b>Events</b> jump to the same panels as on desktop</li>
          </ul>
          <p class="shortcuts-sub">With an external keyboard connected:</p>
        {/if}
        <ul>
          <li><kbd>Space</kbd> Listen or mute the active VFO</li>
          <li><kbd>S</kbd> Skip this hit · <kbd>L</kbd> Lockout · <kbd>H</kbd> Hold or resume</li>
          <li><kbd>←</kbd> <kbd>→</kbd> Fine-tune when the spectrum is focused</li>
          <li><kbd>Shift</kbd> + arrows — 10 kHz steps on the spectrum</li>
          <li><kbd>B</kbd> Show or hide saved frequencies</li>
          <li><kbd>?</kbd> Toggle this help panel</li>
        </ul>
      </section>
    {/if}
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
    {#if showGettingStarted}
      <section class="getting-started card" role="status">
        <div>
          <strong>Ready to listen</strong>
          <p>Pick a <b>voice band</b> above, then press <b>▶ Listen</b> when a VFO shows <b>VOICE</b>. Raise squelch if noise keeps channels open.</p>
        </div>
        <div class="getting-started-actions">
          {#if vfos.length && allVfosMuted}
            <button class="primary" type="button" onclick={() => toggleVfoAudio(vfos[0])}>▶ Start listening</button>
          {/if}
          <button type="button" onclick={dismissOnboarding}>Dismiss</button>
        </div>
      </section>
    {/if}
    <div class="device-strip card">
      <div class="device-name">
        <span class="dot" class:on={connected}></span>
        {deviceLabel}
      </div>
      <div class="receiver-readout">
        <span>Hardware center</span>
        <strong>{displayedCenterHz > 0 ? fmtHz(displayedCenterHz) : 'Not tuned'}</strong>
        <small>Shared RF window — panning moves what every client sees</small>
      </div>
      {#if availableSampleRates().length}
        <label class="span-select">
          <span>Capture span</span>
          <select value={sampleRateHz} onchange={setVisibleSpan} aria-label="SDR capture sample rate">
            {#each availableSampleRates() as rate}<option value={rate}>{rate / 1e6} MSPS</option>{/each}
          </select>
          <small>Applied bandwidth {appliedBandwidthHz > 0 ? fmtHz(appliedBandwidthHz) : 'reading…'} · sample rate ≠ analog bandwidth</small>
        </label>
      {/if}
      <div class="device-meta">
        <span class="vfo-summary">{vfos.length} listening VFO{vfos.length === 1 ? '' : 's'}</span>
        <span class="audio-status" class:on={audioState === 'playing'}>Audio: {audioState}</span>
      </div>
    </div>

    {#if connected && deviceCaps?.connected}
      <section id="rf-controls" class="rf-panel card" class:collapsed={!rfPanelOpen}>
        <div class="rf-panel-head">
          <div>
            <h2>RF controls</h2>
            <p class="rf-panel-help">Gain, AGC, antenna, and device-specific options (Bias-T, preamp) from the driver.</p>
          </div>
          <div class="rf-panel-actions">
            <button type="button" class="mini" onclick={toggleRfPanel} aria-expanded={rfPanelOpen}>{rfPanelOpen ? 'Hide' : 'Show'}</button>
            <button type="button" class="mini" onclick={() => void refreshDeviceCaps()} disabled={rfControlBusy}>Refresh</button>
            <a class="mini-link" href="#/settings">All device settings</a>
          </div>
        </div>
        {#if rfPanelOpen}
          <div class="rf-grid">
            {#if deviceCaps.agc_supported}
              <label class="rf-control">
                <span>RF AGC</span>
                <input
                  type="checkbox"
                  checked={deviceCaps.agc_enabled}
                  disabled={rfControlBusy}
                  onchange={(e) => setDeviceControl('agc', e.currentTarget.checked)}
                />
              </label>
            {/if}
            {#if deviceCaps.antennas?.length > 1}
              <label class="rf-control">
                <span>Antenna</span>
                <select
                  value={deviceCaps.antenna}
                  disabled={rfControlBusy}
                  onchange={(e) => setDeviceControl('antenna', e.currentTarget.value)}
                >
                  {#each deviceCaps.antennas as antenna}
                    <option value={antenna}>{antenna}</option>
                  {/each}
                </select>
              </label>
            {/if}
            {#each deviceCaps.gain_stages ?? [] as stage (stage.name)}
              <label class="rf-control wide">
                <span>
                  {gainStageLabel(stage, deviceDriver)}
                  <small>{stage.value_db.toFixed(1)} dB{#if deviceDriver === 'sdrplay'} · lower reduction = more gain{/if}</small>
                </span>
                <input
                  type="range"
                  min={stage.min_db}
                  max={stage.max_db}
                  step={stage.step_db || 1}
                  value={stage.value_db}
                  disabled={rfControlBusy || deviceCaps.agc_enabled}
                  oninput={(e) => (stage.value_db = Number(e.currentTarget.value))}
                  onchange={(e) => setDeviceControl(`gain:${stage.name}`, e.currentTarget.value)}
                />
              </label>
            {/each}
            {#each commonRfSettings as setting (setting.key)}
              <label class="rf-control">
                <span>{setting.name}</span>
                {#if setting.kind === 'bool'}
                  <input
                    type="checkbox"
                    checked={setting.value === 'true'}
                    disabled={rfControlBusy}
                    onchange={(e) => setDeviceControl(`setting:${setting.key}`, e.currentTarget.checked)}
                  />
                {:else if setting.options?.length}
                  <select
                    value={setting.value}
                    disabled={rfControlBusy}
                    onchange={(e) => setDeviceControl(`setting:${setting.key}`, e.currentTarget.value)}
                  >
                    {#each setting.options as option}
                      <option value={option}>{option}</option>
                    {/each}
                  </select>
                {:else}
                  <input
                    type="number"
                    min={setting.min}
                    max={setting.max}
                    step={setting.step || 1}
                    value={setting.value}
                    disabled={rfControlBusy}
                    onchange={(e) => setDeviceControl(`setting:${setting.key}`, e.currentTarget.value)}
                  />
                {/if}
              </label>
            {/each}
            {#if expertRfSettings.length}
              <details class="rf-expert">
                <summary>More driver settings ({expertRfSettings.length})</summary>
                <div class="rf-grid">
                  {#each expertRfSettings as setting (setting.key)}
                    <label class="rf-control">
                      <span>{setting.name}</span>
                      {#if setting.kind === 'bool'}
                        <input
                          type="checkbox"
                          checked={setting.value === 'true'}
                          disabled={rfControlBusy}
                          onchange={(e) => setDeviceControl(`setting:${setting.key}`, e.currentTarget.checked)}
                        />
                      {:else if setting.options?.length}
                        <select
                          value={setting.value}
                          disabled={rfControlBusy}
                          onchange={(e) => setDeviceControl(`setting:${setting.key}`, e.currentTarget.value)}
                        >
                          {#each setting.options as option}
                            <option value={option}>{option}</option>
                          {/each}
                        </select>
                      {:else}
                        <input
                          type="number"
                          min={setting.min}
                          max={setting.max}
                          step={setting.step || 1}
                          value={setting.value}
                          disabled={rfControlBusy}
                          onchange={(e) => setDeviceControl(`setting:${setting.key}`, e.currentTarget.value)}
                        />
                      {/if}
                    </label>
                  {/each}
                </div>
              </details>
            {/if}
          </div>
        {/if}
      </section>
    {/if}

    <div id="spectrum-display" class="spectrum-wrap card" class:stale={spectrumStale}>
      <div class="spectrum-heading">
        <div class="spectrum-title-block">
          <h2>Live spectrum</h2>
          <p class="fft-status">{spectrumError || (spectrumStale ? 'Reconnecting to FFT stream…' : `${spectrumBins.length} bins · ${Math.max(0, nowMs - lastSpectrumAt)} ms old · ${renderFps} FPS${droppedSpectrumFrames ? ` · ${droppedSpectrumFrames} dropped` : ''}`)}</p>
        </div>
        <div class="spectrum-tools" role="group" aria-label="Spectrum interaction">
          <button class:active={spectrumTool === 'vfo'} onclick={() => (spectrumTool = 'vfo')}>Tune VFO</button>
          <button class:active={spectrumTool === 'center'} onclick={() => (spectrumTool = 'center')}>Pan window</button>
        </div>
      </div>
      <div class="center-controls">
        <button onclick={() => panCenter(-0.25)} aria-label="Move center left by one quarter of the visible span">← ¼ span</button>
        <label><span>Hardware center frequency</span><span class="frequency-entry"><input type="number" min="0.001" step="0.001" value={(displayedCenterHz / 1e6).toFixed(6)} onchange={setCenterFromInput} aria-label="Hardware center frequency in megahertz" /><b>MHz</b></span></label>
        <button onclick={centerOnVfo} disabled={!vfos.length} title="Put VFO 0 in the middle of the visible spectrum">Center on VFO</button>
        <button onclick={() => panCenter(0.25)} aria-label="Move center right by one quarter of the visible span">¼ span →</button>
      </div>
      <p class="interaction-help">{spectrumTool === 'vfo' ? 'Click a peak to tune the listening VFO. Scroll or drag to pan the hardware window.' : 'Click or drag to pan the hardware window without moving the listening VFO.'}</p>
      <canvas class:center-mode={spectrumTool === 'center'} use:bindSpectrumCanvas onclick={tuneFromSpectrum} onwheel={panSurfaceWheel} onpointerdown={beginSurfacePan} onpointermove={moveSurfacePan} onpointerup={endSurfacePan} onpointercancel={endSurfacePan} onkeydown={tuneFromSpectrumKeyboard} tabindex="0" role="slider" aria-valuemin="0" aria-valuemax={Math.max(1, sampleRateHz)} aria-valuenow={spectrumTool === 'center' ? displayedCenterHz : (vfos[0]?.frequency_hz ?? centerFreqHz)} aria-label={spectrumTool === 'center' ? 'Spectrum center control. Click or drag to change center frequency; use arrow keys for fine adjustment.' : 'Spectrum tuner. Click to tune VFO 0; scroll to pan center frequency; use arrow keys to fine tune.'} title={spectrumTool === 'center' ? 'Click or drag to move the receiver center' : 'Click to tune VFO 0; scroll to pan'}></canvas>
      {#if spectrumStale}<div class="stale-overlay">Spectrum reconnecting</div>{/if}
      <div class="waterfall-head">
        <div class="waterfall-title-block">
          <h2 class="waterfall-title">Waterfall</h2>
          <p class="waterfall-sub">openWebRX-style levels · newest row at top · yellow trace = peak hold</p>
        </div>
        <div class="display-controls">
          <label class="display-field"><span>Min dB</span><input type="number" step="5" value={displayConfig.minDb} onchange={setDisplayMinDb} aria-label="Spectrum minimum dB" /></label>
          <label class="display-field"><span>Max dB</span><input type="number" step="5" value={displayConfig.maxDb} onchange={setDisplayMaxDb} aria-label="Spectrum maximum dB" /></label>
          <button type="button" class="mini" onclick={autoSpectrumLevels} title="Auto-adjust levels like openWebRX">Auto levels</button>
          <label class="display-field"><span>Smooth</span><input type="range" min="0" max="0.85" step="0.05" value={displayConfig.smoothing} oninput={setDisplaySmoothing} aria-label="Spectrum smoothing" /></label>
          <label class="display-field"><span>Intensity</span><input aria-label="Waterfall intensity boost" type="range" min="0.25" max="4" step="0.25" value={waterfallGain} oninput={setWaterfallGain} /></label>
          <select aria-label="Waterfall palette" value={displayConfig.palette} onchange={setDisplayPalette}>
            <option value="openwebrx">OpenWebRX</option>
            <option value="classic">Classic</option>
            <option value="mono">Mono</option>
          </select>
          <label class="display-toggle"><input type="checkbox" checked={displayConfig.peakHold} onchange={(e) => { displayConfig.peakHold = e.currentTarget.checked; peakHold.reset(); persistDisplayConfig(); }} /> Peak</label>
        </div>
      </div>
      <div class="waterfall-stage">
        <canvas class="waterfall" class:center-mode={spectrumTool === 'center'} use:bindWaterfallCanvas onclick={tuneFromSpectrum} onwheel={panSurfaceWheel} onpointerdown={beginSurfacePan} onpointermove={moveSurfacePan} onpointerup={endSurfacePan} onpointercancel={endSurfacePan} aria-label="Live waterfall. Scroll to pan the receiver center; select Pan window to click or drag."></canvas>
        {#if !connected}
          <div class="waterfall-empty">Connect an SDR to see the waterfall.</div>
        {:else if spectrumBins.length === 0}
          <div class="waterfall-empty">Waiting for FFT frames from the receiver…</div>
        {:else if spectrumStale}
          <div class="waterfall-empty warn">Spectrum stream paused — showing last known frames.</div>
        {/if}
      </div>
      {#if scanRunning}
        <div class="scan-progress" role="status"><span><b>Scanning {activeBank?.name ?? activeRange ?? 'range'}</b><small>{scanLocked ? 'HOLD until Resume' : scanHolding ? 'delay — resumes when the channel closes' : activeBank ? `${fmtHz(activeBank.start_hz)} – ${fmtHz(activeBank.end_hz)}` : 'channel scan'}</small></span><div class="scan-track"><i style={`width:${scanProgress}%`}></i></div><strong>{scanProgress.toFixed(0)}%</strong></div>
      {/if}
    </div>

    <section class="signal-history card">
      <div class="history-head"><div><h2>Found signals</h2><small>Scanner hits assigned to VFO slots during the current survey.</small></div><button class="mini" onclick={async () => (signalHistory = await Api.signalEvents(100))}>Refresh</button></div>
      {#if foundSignals.length === 0}
        <div class="history-empty">No signals found in this survey yet.</div>
      {:else}
        <div class="history-list">
          {#each foundSignals as hit}
            <div class="history-row"><span>{hit.timestamp_ms ? fmtTime(hit.timestamp_ms) : 'live'}</span><b>{fmtHz(Number(hit.frequency_hz ?? 0))}</b><span>{hit.sub_protocol ?? hit.signal_class ?? hit.family ?? 'signal'}</span><span>SNR {Number(hit.snr_db ?? 0).toFixed(1)} dB</span><button class="mini listen-hit" onclick={() => tuneFoundSignal(hit)}>Center & listen</button></div>
          {/each}
        </div>
      {/if}
    </section>

    <div class="vfo-grid">
      {#each vfos as v (v.id)}
        <div class="vfo-tile card">
          <div class="vfo-freq">{fmtHz(v.frequency_hz)}</div>
          <small class="vfo-offset">Listening VFO · {v.frequency_hz >= centerFreqHz ? '+' : '−'}{fmtHz(Math.abs(v.frequency_hz - centerFreqHz))} from center</small>
          {#if v.locked}<span class="vfo-lock">LOCKED · logged {v.last_hit_ms ? fmtTime(v.last_hit_ms) : 'now'}</span>{/if}
          <div class="vfo-controls">
            <input class="freq-input" type="number" step="100" value={v.frequency_hz} aria-label="VFO {v.id} frequency" onchange={(e) => setVfoFrequency(v.id, e)} />
            <select aria-label="VFO {v.id} mode" value={v.mode} onchange={(e) => setVfoMode(v.id, e)}>
              <option value="nfm">NFM</option><option value="wfm">WFM</option><option value="am">AM</option><option value="lsb">LSB</option><option value="usb">USB</option>
            </select>
          </div>
          <div class="vfo-mode">{v.mode.toUpperCase()} · VFO {v.id}</div>
          <div class="vfo-signal-head">
            <span class="signal-dot" class:on={v.squelch_open}></span>
            <span class="squelch-badge" class:open={v.squelch_open}>{v.squelch_open ? 'VOICE' : 'quiet'}</span>
            <span>SNR {(v.snr_db ?? v.strength_db - (v.noise_floor_db ?? noiseFloorDb)).toFixed(0)} dB</span>
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
  <aside id="events-log" class="log card" class:expanded={logExpanded}>
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
    grid-template-columns: 230px 1fr;
    grid-template-rows: minmax(0, 1fr) 140px;
    gap: 8px;
    height: 100%;
    padding: 8px;
    overflow: hidden;
  }
  .scanner-layout.banks-collapsed { grid-template-columns: 52px 1fr; }
  #band-presets, #rf-controls, #spectrum-display, #events-log, #scan-workspace { scroll-margin-top: 64px; }
  .banks {
    display: flex; flex-direction: column; gap: 8px;
    overflow: hidden;
    background: var(--bg-elev);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 8px;
    min-width: 0;
  }
  .banks.collapsed { padding: 6px; align-items: stretch; }
  .banks-rail { display: flex; align-items: flex-start; }
  .panel-toggle {
    min-height: 30px;
    padding: 5px 8px;
    font-size: 11px;
    border-color: var(--line);
    background: var(--bg);
    color: var(--fg-dim);
    white-space: nowrap;
  }
  .banks.collapsed .banks-toggle {
    width: 100%;
    min-height: 44px;
    writing-mode: vertical-rl;
    text-orientation: mixed;
    transform: rotate(180deg);
    padding: 10px 4px;
    letter-spacing: 0.04em;
    font: 600 10px var(--mono);
    text-transform: uppercase;
  }
  .layout-toggle {
    min-height: 28px;
    padding: 4px 8px;
    font-size: 11px;
    border-color: var(--line);
    background: var(--bg);
    color: var(--fg-dim);
  }
  .layout-toggle[aria-pressed='true'] { color: var(--accent); border-color: var(--accent); }
  .shortcuts-card { padding: 10px 12px; border-color: rgba(45, 212, 191, 0.35); background: rgba(45, 212, 191, 0.06); }
  .shortcuts-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-bottom: 6px; }
  .shortcuts-card ul { margin: 0; padding: 0; list-style: none; display: grid; gap: 4px; color: var(--fg-dim); font-size: 12px; }
  .shortcuts-sub { margin: 8px 0 4px; color: var(--fg-dim); font-size: 11px; }
  .mobile-workspace-bar {
    display: none;
    gap: 6px;
    padding: 8px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .mobile-workspace-bar button {
    flex: 1 0 auto;
    min-height: 44px;
    min-width: 72px;
    padding: 8px 12px;
    font-size: 13px;
  }
  .mobile-workspace-bar button.active {
    color: #03120f;
    background: var(--accent);
    border-color: var(--accent);
  }
  .shortcuts-card kbd {
    display: inline-block;
    min-width: 1.4em;
    padding: 1px 5px;
    border: 1px solid var(--line-strong);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    font: 10px var(--mono);
    text-align: center;
  }
  .getting-started {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    border-color: rgba(45, 212, 191, 0.35);
    background: rgba(45, 212, 191, 0.08);
  }
  .getting-started strong { color: var(--accent); }
  .getting-started p { margin: 4px 0 0; color: var(--fg-dim); font-size: 12px; line-height: 1.45; max-width: 52rem; }
  .getting-started-actions { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .banks-header h2 { margin: 0 0 4px; font-size: 12px; color: var(--fg-dim); text-transform: uppercase; letter-spacing: 0.05em; }
  .banks-help { margin: 0 0 8px; color: var(--fg-dim); font-size: 11px; line-height: 1.35; }
  .bank-groups { overflow-y: auto; flex: 1; }
  .bank-group h3 { margin: 10px 4px 4px; color: var(--accent-2); font: 10px var(--mono); text-transform: uppercase; letter-spacing: .08em; }
  .bank-group ul { list-style: none; margin: 0; padding: 0; }
  .empty-banks { color: var(--fg-dim); font-size: 12px; padding: 12px 4px; }
  .bookmark-section { border-top:1px solid var(--line); margin-top:10px; padding-top:10px; }
  .bookmark-section>h3 { margin:0 0 7px; color:var(--fg-dim); font-size:10px; text-transform:uppercase; letter-spacing:.08em; }
  .bookmark-add { display:grid; grid-template-columns:minmax(0,1fr) auto; gap:5px; }
  .bookmark-add input { min-width:0; }
  .bookmark-list { list-style:none; margin:7px 0 0; padding:0; }
  .bookmark-list li { display:grid; grid-template-columns:minmax(0,1fr) auto; gap:4px; align-items:center; }
  .bookmark-tune { display:grid; gap:1px; text-align:left; min-width:0; background:transparent; border-color:transparent; padding:6px; }
  .bookmark-tune span { overflow:hidden; text-overflow:ellipsis; }
  .bookmark-tune small { color:var(--fg-dim); font:9px var(--mono); }
  .bookmark-delete { width:30px; padding:4px; color:var(--fg-dim); background:transparent; border-color:transparent; }
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
  .scan-workspace { border-color: rgba(45, 212, 191, 0.22); background: linear-gradient(180deg, rgba(45, 212, 191, 0.07), var(--bg-elev)); }
  .scan-workspace.collapsed .starter-bands,
  .scan-workspace.collapsed .scan-toolbar,
  .scan-workspace.collapsed .all-bands-panel { display: none; }
  .starter-bands {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 10px;
  }
  .starter-bands.compact { gap: 6px; }
  .band-chip {
    display: grid;
    gap: 2px;
    min-width: 132px;
    padding: 10px 12px;
    text-align: left;
    border: 1px solid var(--line-strong);
    border-radius: 8px;
    background: var(--bg);
  }
  .band-chip strong { font-size: 13px; color: var(--fg); }
  .band-chip small { color: var(--fg-dim); font: 10px var(--mono); }
  .band-chip:hover { border-color: var(--accent-2); }
  .band-chip.active { border-color: var(--accent); background: rgba(45, 212, 191, 0.12); }
  .scan-toolbar { display: grid; gap: 10px; }
  .toggle-field {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    color: var(--fg-dim);
    font-size: 12px;
  }
  .toggle-field input { margin-top: 2px; }
  .toggle-field small { display: block; color: var(--fg-dim); font-size: 10px; }
  .scan-live-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    gap: 10px 14px;
  }
  .squelch-field {
    display: grid;
    gap: 4px;
    flex: 1 1 220px;
    min-width: 200px;
    font-size: 11px;
    color: var(--fg-dim);
  }
  .squelch-field b { color: var(--accent); font: 11px var(--mono); }
  .noise-readout { font: 11px var(--mono); color: var(--fg-dim); padding: 8px 10px; border: 1px solid var(--line); border-radius: 6px; background: var(--bg); }
  .scan-search { display: grid; grid-template-columns: 1fr auto auto auto; gap: 8px; align-items: center; }
  .scan-actions { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .scan-actions button.active { border-color: var(--accent); color: var(--accent); }
  .all-bands-panel { margin-top: 10px; max-height: 280px; overflow: auto; border-top: 1px solid var(--line); padding-top: 10px; }
  .status-strip { padding: 8px 10px; }
  .bookmarks-aside .bookmark-primary h2 { margin: 0 0 4px; font-size: 12px; color: var(--fg-dim); text-transform: uppercase; letter-spacing: 0.05em; }
  .squelch-badge {
    padding: 1px 6px;
    border-radius: 4px;
    border: 1px solid var(--line-strong);
    font: 700 9px var(--mono);
    text-transform: uppercase;
  }
  .squelch-badge.open { color: var(--ok); border-color: var(--ok); background: rgb(34 197 94 / 12%); }
  .command-strip { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 8px 10px; border-radius: 6px; background: var(--bg-elev-2); border: 1px solid var(--line); }
  .quick-modes, .runtime-status { display: flex; align-items: center; gap: 5px; flex-wrap: wrap; }
  .strip-label { color: var(--fg-dim); text-transform: uppercase; font: 10px var(--mono); margin-right: 4px; }
  .quick { padding: 4px 7px; font-size: 11px; border-radius: 3px; }
  .status-pill { color: var(--fg-dim); font: 10px var(--mono); }
  .status-pill.on { color: var(--ok); }
  .settings-link { color: var(--fg); text-decoration: none; font-size: 12px; border-left: 1px solid var(--line-strong); padding-left: 8px; }

  .center { display: flex; flex-direction: column; gap: 8px; overflow-y: auto; min-height: 0; padding-right: 2px; }
  .device-strip { display: flex; justify-content: space-between; align-items: center; gap: 16px; padding: 10px 12px; font-size: 13px; flex-wrap: wrap; }
  .device-name { display: flex; align-items: center; min-width: 140px; }
  .device-meta { display: flex; flex-direction: column; gap: 2px; align-items: flex-end; text-align: right; }
  .rf-panel { padding: 10px 12px; }
  .rf-panel.collapsed .rf-grid { display: none; }
  .rf-panel-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; margin-bottom: 8px; }
  .rf-panel-head h2 { margin: 0; font-size: 12px; text-transform: uppercase; letter-spacing: 0.05em; color: var(--fg-dim); }
  .rf-panel-help { margin: 4px 0 0; color: var(--fg-dim); font-size: 11px; max-width: 42rem; }
  .rf-panel-actions { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .mini-link { color: var(--accent-2); font-size: 11px; text-decoration: none; padding: 4px 8px; border: 1px solid var(--line); border-radius: 4px; }
  .rf-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 8px 12px; align-items: end; }
  .rf-control { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--fg-dim); }
  .rf-control.wide { grid-column: span 2; }
  .rf-control span small { display: block; color: var(--fg-dim); font: 9px var(--mono); }
  .rf-control select, .rf-control input[type='number'] { min-height: 32px; font-size: 12px; }
  .rf-expert { grid-column: 1 / -1; color: var(--fg-dim); font-size: 12px; }
  .rf-expert summary { cursor: pointer; padding: 6px 0; }
  .display-controls { display: flex; flex-wrap: wrap; align-items: end; gap: 6px; }
  .display-field { display: flex; flex-direction: column; gap: 2px; font: 9px var(--mono); color: var(--fg-dim); }
  .display-field input[type='number'] { width: 72px; min-height: 30px; padding: 4px 6px; font-size: 11px; }
  .display-field input[type='range'] { width: 72px; }
  .display-toggle { display: flex; align-items: center; gap: 4px; font: 10px var(--mono); color: var(--fg-dim); min-height: 30px; }
  .display-controls select { min-height: 30px; font-size: 11px; }
  .rf-control input[type='range'] { min-height: 32px; }
  .receiver-readout { display: flex; flex-direction:column; align-items:flex-start; gap: 2px; font-family: var(--mono); }
  .receiver-readout span, .receiver-readout small { color: var(--fg-dim); font-size: 10px; }
  .receiver-readout strong { color: var(--accent); font-size: 14px; }
  .span-select { display:flex; flex-direction:column; gap:2px; color:var(--fg-dim); font:9px var(--mono); }
  .span-select select { min-height:30px; padding:4px 7px; font-size:11px; }
  .span-select small { max-width:260px; color:var(--fg-dim); font:9px/1.3 var(--mono); }
  .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; background: var(--danger); margin-right: 8px; }
  .dot.on { background: var(--ok); box-shadow: 0 0 6px var(--ok); }
  .vfo-summary { color: var(--fg-dim); }
  .audio-status { color: var(--fg-dim); text-transform: capitalize; }
  .audio-status.on { color: var(--ok); }

  .spectrum-wrap { flex: 0 0 auto; min-height: 560px; position: relative; }
  .spectrum-wrap.stale canvas { opacity: 0.45; }
  .stale-overlay { position:absolute; inset:36px 12px 112px; display:grid; place-items:center; color:var(--warn); background:rgb(7 12 18 / 55%); font:700 12px var(--mono); text-transform:uppercase; letter-spacing:.08em; pointer-events:none; }
  .spectrum-wrap canvas { display: block; width: 100%; height: 170px; background: #070c12; border-radius: 4px; }
  .spectrum-wrap canvas { cursor:crosshair; }
  .spectrum-wrap canvas.center-mode { cursor:grab; touch-action:none; }
  .spectrum-wrap canvas.center-mode:active { cursor:grabbing; }
  .spectrum-heading { display:flex; align-items:flex-start; justify-content:space-between; gap:10px; }
  .spectrum-title-block h2 { margin:0; }
  .spectrum-title-block .fft-status { margin:2px 0 0; color:var(--fg-dim); font:10px var(--mono); }
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
  .waterfall-title { margin: 0 !important; }
  .waterfall-title-block .waterfall-sub { margin:2px 0 0; color:var(--fg-dim); font:10px var(--mono); }
  .waterfall-head { display:flex; align-items:flex-end; gap:8px; margin-top:8px; }
  .waterfall-head .waterfall-title-block { flex:1; }
  .waterfall-head label, .waterfall-head select { font:10px var(--mono); color:var(--fg-dim); }
  .waterfall-head input { width:70px; vertical-align:middle; }
  .waterfall-stage { position:relative; }
  .waterfall-empty { position:absolute; inset:0; display:grid; place-items:center; padding:16px; text-align:center; color:var(--fg-dim); background:rgb(7 12 18 / 72%); font:12px var(--sans); pointer-events:none; border-radius:4px; }
  .waterfall-empty.warn { color:var(--warn); }
  .spectrum-wrap canvas.waterfall { height: clamp(320px, 48vh, 620px); image-rendering: auto; }
  .scan-progress { display:grid; grid-template-columns:auto 1fr 38px; align-items:center; gap:10px; margin-top:8px; padding:7px 9px; border:1px solid var(--line); border-radius:6px; background:var(--bg); }
  .scan-progress > span { display:flex; flex-direction:column; font-size:11px; }
  .scan-progress small { color:var(--fg-dim); font:9px var(--mono); }
  .scan-progress > strong { color:var(--accent); font:11px var(--mono); text-align:right; }
  .scan-track { height:6px; overflow:hidden; border-radius:4px; background:var(--line); }
  .scan-track i { display:block; height:100%; background:var(--accent); transition:width .2s linear; }

  .signal-history { order: 3; max-height: 190px; overflow: hidden; }
  .history-head { display: flex; justify-content: space-between; align-items: center; }
  .history-head h2 { margin-bottom: 0; }
  .history-head small { color:var(--fg-dim); font-size:10px; }
  .history-list { overflow-y: auto; max-height: 145px; }
  .history-row { display: grid; grid-template-columns: 70px 100px minmax(90px,1fr) 85px auto; align-items:center; gap: 8px; padding: 5px 0; border-top: 1px solid var(--line); color: var(--fg-dim); font: 10px var(--mono); }
  .history-row b { color: var(--accent); }
  .listen-hit { white-space:nowrap; color:var(--accent); border-color:var(--accent); }
  .history-empty { color: var(--fg-dim); padding: 8px 0; font-size: 12px; }

  .vfo-grid { order: 2; display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 8px; }
  .vfo-tile { display: flex; flex-direction: column; gap: 4px; }
  .vfo-freq { font-family: var(--mono); font-size: 18px; font-weight: 600; color: var(--accent); }
  .vfo-offset { color:var(--fg-dim); font:9px var(--mono); }
  .vfo-lock { align-self:flex-start; padding:2px 5px; border:1px solid var(--ok); border-radius:3px; color:var(--ok); background:rgb(34 197 94 / 10%); font:700 9px var(--mono); }
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
  .scanner-layout.log-expanded .log { display: block; }
  .dock-tabs { display: flex; align-items: center; gap: 3px; padding: 4px 8px; background: var(--bg-elev-2); border-bottom: 1px solid var(--line-strong); }
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
    .scanner-layout.compact .mobile-workspace-bar { display: flex; }
    #band-presets, #rf-controls, #spectrum-display, #events-log { scroll-margin-top: 128px; }
    .banks { max-height:none; order:3; padding:10px; }
    .banks.collapsed { flex-direction: row; align-items: center; padding: 8px 10px; }
    .banks.collapsed .banks-toggle {
      writing-mode: horizontal-tb;
      transform: none;
      width: 100%;
      min-height: 44px;
      padding: 10px 12px;
      font-size: 13px;
      text-transform: none;
    }
    .banks-header { display:grid; grid-template-columns:1fr; gap:6px; }
    .banks-header input { min-height:42px; font-size:14px; }
    .bank-groups { max-height:280px; }
    .range-row { min-height:52px; padding:9px 10px; }
    .range-name { font-size:14px; }
    .center { order:1; overflow:visible; padding:0; gap:8px; }
    #band-presets, #rf-controls, #spectrum-display, #events-log, #scan-workspace { scroll-margin-top: 128px; }
    .band-chip { min-width: 0; flex: 1 1 calc(50% - 8px); min-height: 52px; }
    .starter-bands { flex-wrap: wrap; }
    .scan-live-controls { flex-direction: column; align-items: stretch; }
    .scan-search { grid-template-columns: 1fr; }
    .status-strip { position: sticky; top: 0; z-index: 9; box-shadow: 0 4px 14px rgba(0,0,0,.2); }
    .scan-workspace { order: -2; }
    .runtime-status {
      display: flex;
      flex-wrap: nowrap;
      gap: 6px;
      overflow-x: auto;
      scrollbar-width: none;
      padding-bottom: 2px;
    }
    .runtime-status .status-pill { flex: 0 0 auto; font-size: 10px; }
    .runtime-status .layout-toggle { display: none; }
    .settings-link { margin-left: 0; flex: 0 0 auto; min-height: 40px; display: inline-flex; align-items: center; }
    .layout-toggle, .panel-toggle, .mini, .mini-link { min-height: 44px; }
    .setup-card { align-items:flex-start; flex-direction:column; }
    .device-strip { gap:8px; flex-wrap:wrap; padding:11px; }
    .device-strip > div:first-child { width:100%; font-size:14px; }
    .span-select { width:100%; order:1; }
    .span-select select { min-height:42px; width:100%; font-size:14px; }
    .vfo-summary { margin-left:auto; }
    .audio-status { width:100%; padding-top:6px; border-top:1px solid var(--line); }
    .receiver-readout { order:-1; width:100%; }
    .rf-panel-head { flex-direction: column; align-items: stretch; gap: 8px; }
    .rf-panel-actions { justify-content: flex-start; flex-wrap: wrap; }
    .rf-control input[type='range'] { min-height: 44px; }
    .rf-control select, .rf-control input[type='number'] { min-height: 44px; font-size: 14px; }
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
    .spectrum-wrap canvas { height:140px; }
    .spectrum-wrap canvas.waterfall { height:max(280px, 42vh); }
    .waterfall-head { flex-direction: column; align-items: stretch; gap: 8px; }
    .display-controls {
      display: flex;
      flex-wrap: nowrap;
      align-items: end;
      gap: 8px;
      overflow-x: auto;
      scrollbar-width: none;
      width: 100%;
      padding-bottom: 4px;
    }
    .display-field input[type='number'] { min-height: 44px; width: 84px; font-size: 14px; }
    .display-field input[type='range'] { width: 96px; min-height: 44px; }
    .display-controls select { min-height: 44px; font-size: 13px; }
    .display-controls .mini { min-height: 44px; flex: 0 0 auto; }
    .display-toggle { min-height: 44px; }
    .rf-grid { grid-template-columns: 1fr; }
    .rf-control.wide { grid-column: auto; }
    .vfo-grid { grid-template-columns:1fr; gap:10px; }
    .vfo-tile { padding:12px; gap:8px; }
    .vfo-freq { font-size:22px; }
    .vfo-bar .listen { min-width:110px; min-height:44px; font-size:14px; }
    .signal-history { display:block; max-height:none; order:2; }
    .history-list { max-height:260px; }
    .history-row { grid-template-columns:1fr auto; gap:4px 8px; padding:9px 0; }
    .history-row > span:first-child { display:none; }
    .history-row .listen-hit { grid-column:1 / -1; min-height:40px; }
    .log { display:none; }
    .scanner-layout.log-expanded .log { display:block; order:4; min-height:220px; }
    .getting-started { flex-direction:column; align-items:flex-start; }
    .getting-started-actions button { min-height: 44px; }
  }

  /* Short laptop / WebView windows: live RF controls beat an empty log dock. */
  @media (max-height: 850px) {
    .scanner-layout { grid-template-rows: minmax(0, 1fr); }
    .log { display: none; }
    .scanner-layout.log-expanded .log { display: block; grid-row: auto; min-height: 140px; }
    .spectrum-wrap { min-height: 430px; }
    .spectrum-wrap canvas { height: 110px; }
    .spectrum-wrap canvas.waterfall { height: 260px; }
    .vfo-tile { padding: 8px; }
    .signal-history { display: none; }
  }
</style>
