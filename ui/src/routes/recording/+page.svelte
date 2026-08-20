<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  let iq = $state<any>({});
  let playback = $state<any>({ playing: false });
  let recordings = $state<any[]>([]);
  let transcription = $state<any>({});
  let annotations = $state<any[]>([]);
  let error = $state('');
  let busy = $state(false);
  let seekPct = $state(0);
  let iqRunning = $derived(Boolean(iq.recording ?? iq.running ?? iq.active));
  let playing = $derived(Boolean(playback?.playing));

  function fmtBytes(bytes: number) {
    if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
    if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
    if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)} kB`;
    return `${bytes} B`;
  }

  function fmtHz(hz: number) {
    if (hz >= 1e6) return `${(hz / 1e6).toFixed(4)} MHz`;
    if (hz >= 1e3) return `${(hz / 1e3).toFixed(1)} kHz`;
    return `${hz} Hz`;
  }

  async function load() {
    try {
      const [nextIq, nextAnnotations, nextTranscription, recs] = await Promise.all([
        Api.iqRecordingStatus(),
        Api.recordingAnnotations(),
        Api.transcriptionStatus(),
        Api.recordingsV2(),
      ]);
      iq = nextIq;
      playback = recs?.playback ?? { playing: false };
      recordings = Array.isArray(recs?.recordings) ? recs.recordings : [];
      annotations = Array.isArray(nextAnnotations)
        ? nextAnnotations
        : ((nextAnnotations as { annotations?: any[] } | null)?.annotations ?? []);
      transcription = nextTranscription;
      error = '';
      if (playback?.progress != null) seekPct = Math.round(Number(playback.progress) * 100);
    } catch (e) {
      error = String(e);
    }
  }

  async function iqToggle() {
    busy = true;
    try {
      if (iqRunning) await Api.iqRecordingStop();
      else await Api.iqRecordingStart();
      await load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function startPlayback(path: string) {
    busy = true;
    try {
      await Api.playbackStart(path);
      await load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function stopPlayback() {
    busy = true;
    try {
      await Api.playbackStop();
      await load();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function seekPlayback() {
    const total = Number(playback?.total_samples ?? 0);
    if (!total) return;
    const offset = Math.round((seekPct / 100) * total);
    try {
      const result = await Api.playbackSeek(offset);
      playback = result?.status ?? playback;
      if (result?.status?.progress != null) seekPct = Math.round(Number(result.status.progress) * 100);
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    void load();
    const timer = window.setInterval(() => {
      if (playing) void load();
    }, 2000);
    return () => window.clearInterval(timer);
  });
</script>

<svelte:head><title>Recordings · PulseScope</title></svelte:head>
<div class="page">
  <h1>Recording</h1>
  <p class="muted">Capture bounded CF32 IQ, replay through the live receiver waterfall, and annotate captures.</p>
  {#if error}<p class="error">{error}</p>{/if}

  <div class="grid">
    <section class="card">
      <h2>IQ recording</h2>
      <p class:ok={iqRunning}>{iqRunning ? 'Recording' : 'Stopped'}</p>
      <dl>
        <div><dt>Format</dt><dd>{iq.format ?? 'cf32-le'}</dd></div>
        <div><dt>Samples</dt><dd>{Number(iq.samples_written ?? 0).toLocaleString()}</dd></div>
        <div><dt>Size</dt><dd>{fmtBytes(Number(iq.bytes_written ?? 0))}</dd></div>
        <div><dt>Center</dt><dd>{fmtHz(Number(iq.center_freq_hz ?? 0))}</dd></div>
        <div><dt>Rate</dt><dd>{Number(iq.sample_rate_hz ?? 0).toLocaleString()} Hz</dd></div>
      </dl>
      {#if iq.write_error}<p class="error">{iq.write_error}</p>{/if}
      <button class="primary" onclick={iqToggle} disabled={busy}>{iqRunning ? 'Stop recording' : 'Start recording'}</button>
    </section>

    <section class="card">
      <h2>Replay</h2>
      <p class:ok={playing}>{playing ? 'Replaying into receiver' : 'Idle'}</p>
      {#if playing}
        <label class="seek">
          <span>Position {seekPct}%</span>
          <input type="range" min="0" max="100" bind:value={seekPct} onchange={seekPlayback} />
        </label>
        <p class="muted">{Number(playback?.samples_read ?? 0).toLocaleString()} / {Number(playback?.total_samples ?? 0).toLocaleString()} samples</p>
        <button onclick={stopPlayback} disabled={busy}>Stop replay</button>
        <a href="#/" class="link">Open Receiver waterfall</a>
      {:else}
        <p class="muted">Select a capture below to feed IQ through the same bounded capture path used for live RF.</p>
      {/if}
    </section>

    <section class="card beta">
      <h2>Transcription <span>Beta</span></h2>
      <p class="muted">{transcription.missing_gate ?? transcription.install_hint ?? 'Local whisper.cpp engine is optional.'}</p>
      <p>Status: {transcription.available ? (transcription.running ? 'running' : 'ready') : 'unavailable'}</p>
      {#if transcription.available}
        <button class="primary" disabled={busy} onclick={async () => { busy = true; try { if (transcription.running) await Api.transcriptionStop(); else await Api.transcriptionStart(); await load(); } catch (e) { error = String(e); } finally { busy = false; } }}>
          {transcription.running ? 'Stop transcription' : 'Start transcription'}
        </button>
      {:else}
        <button disabled title={transcription.install_hint ?? 'whisper.cpp is not installed'}>Start transcription</button>
      {/if}
    </section>
  </div>

  <section class="card">
    <h2>Captured IQ files</h2>
    {#each recordings as rec}
      <div class="rec-row">
        <div>
          <b>{rec.name}</b>
          <small>{fmtBytes(Number(rec.size_bytes ?? 0))} · {Number(rec.samples ?? 0).toLocaleString()} samples</small>
          {#if rec.metadata}
            <small>{fmtHz(Number(rec.metadata.center_freq_hz ?? 0))} · {Number(rec.metadata.sample_rate_hz ?? 0).toLocaleString()} Hz</small>
          {/if}
        </div>
        <button class="primary" disabled={busy || playing} onclick={() => startPlayback(rec.path ?? rec.name)}>Replay</button>
      </div>
    {:else}
      <div class="empty">No recordings yet.</div>
    {/each}
  </section>

  <button onclick={load}>Refresh</button>

  <section class="card">
    <h2>Annotations</h2>
    {#each annotations as item}
      <div class="row">{typeof item === 'object' ? `${item.offset_ms ?? ''} ${item.text ?? JSON.stringify(item)}` : item}</div>
    {:else}
      <div class="empty">No annotations</div>
    {/each}
  </section>
</div>

<style>
  .page { padding: 16px; overflow-y: auto; height: 100%; }
  .muted, .empty { color: var(--fg-dim); }
  .error { color: var(--danger); }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 12px; }
  .card { margin: 12px 0; padding: 14px; background: var(--bg-elev); border: 1px solid var(--line); border-radius: 8px; }
  h2 { font-size: 14px; margin-top: 0; display: flex; gap: 8px; align-items: center; }
  h2 span { font: 10px var(--mono); color: var(--warn); border: 1px solid var(--warn); border-radius: 4px; padding: 1px 6px; }
  .ok { color: var(--ok); }
  .beta button:disabled { opacity: .55; cursor: not-allowed; }
  button { background: var(--bg); color: var(--fg); border: 1px solid var(--line); padding: 7px 11px; border-radius: 5px; }
  .primary { color: var(--accent); border-color: var(--accent); }
  dl { display: grid; gap: 6px; margin: 10px 0; font: 12px var(--mono); }
  dl div { display: flex; justify-content: space-between; gap: 12px; }
  dt { color: var(--fg-dim); }
  dd { margin: 0; word-break: break-all; }
  .seek { display: grid; gap: 6px; margin: 10px 0; font-size: 12px; }
  .link { color: var(--accent-2); font-size: 12px; }
  .rec-row { display: grid; grid-template-columns: 1fr auto; gap: 12px; align-items: center; padding: 8px 0; border-top: 1px solid var(--line); }
  .rec-row small { display: block; color: var(--fg-dim); font-size: 11px; }
  .row { font: 12px var(--mono); padding: 8px 0; border-top: 1px solid var(--line); word-break: break-word; }
  .empty { text-align: center; padding: 16px; }
</style>
