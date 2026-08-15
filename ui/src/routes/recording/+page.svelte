<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  let iq = $state<any>({});
  let transcription = $state<any>({});
  let annotations = $state<any[]>([]);
  let error = $state('');
  let busy = $state(false);
  let iqRunning = $derived(Boolean(iq.recording ?? iq.running ?? iq.active));

  function fmtBytes(bytes: number) {
    if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
    if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
    if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)} kB`;
    return `${bytes} B`;
  }

  async function load() {
    try {
      const [nextIq, nextAnnotations, nextTranscription] = await Promise.all([
        Api.iqRecordingStatus(),
        Api.recordingAnnotations(),
        Api.transcriptionStatus(),
      ]);
      iq = nextIq;
      annotations = Array.isArray(nextAnnotations)
        ? nextAnnotations
        : ((nextAnnotations as { annotations?: any[] } | null)?.annotations ?? []);
      transcription = nextTranscription;
      error = '';
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

  onMount(load);
</script>

<svelte:head><title>Recordings · PulseScope</title></svelte:head>
<div class="page">
  <h1>Recording</h1>
  <p class="muted">Capture bounded CF32 IQ from the live receiver. Speech transcription is not available yet.</p>
  {#if error}<p class="error">{error}</p>{/if}
  <div class="grid">
    <section class="card">
      <h2>IQ recording</h2>
      <p class:ok={iqRunning}>{iqRunning ? 'Recording' : 'Stopped'}</p>
      <dl>
        <div><dt>Format</dt><dd>{iq.format ?? 'cf32-le'}</dd></div>
        <div><dt>Samples</dt><dd>{Number(iq.samples_written ?? 0).toLocaleString()}</dd></div>
        <div><dt>Size</dt><dd>{fmtBytes(Number(iq.bytes_written ?? 0))}</dd></div>
        <div><dt>Path</dt><dd>{iq.path ?? '—'}</dd></div>
      </dl>
      {#if iq.write_error}<p class="error">{iq.write_error}</p>{/if}
      <button class="primary" onclick={iqToggle} disabled={busy}>{iqRunning ? 'Stop recording' : 'Start recording'}</button>
    </section>
    <section class="card beta">
      <h2>Transcription <span>Beta</span></h2>
      <p class="muted">{transcription.missing_gate ?? 'Speech transcription transport is not implemented.'}</p>
      <p>Status: unavailable</p>
      <button disabled title="Transcription is not implemented">Start transcription</button>
    </section>
  </div>
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
  .page{padding:16px;overflow-y:auto;height:100%}
  .muted,.empty{color:var(--fg-dim)}
  .error{color:var(--danger)}
  .grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:12px}
  .card{margin:12px 0;padding:14px;background:var(--bg-elev);border:1px solid var(--line);border-radius:8px}
  h2{font-size:14px;margin-top:0;display:flex;gap:8px;align-items:center}
  h2 span{font:10px var(--mono);color:var(--warn);border:1px solid var(--warn);border-radius:4px;padding:1px 6px}
  .ok{color:var(--ok)}
  .beta button{opacity:.55;cursor:not-allowed}
  button{background:var(--bg);color:var(--fg);border:1px solid var(--line);padding:7px 11px;border-radius:5px}
  .primary{color:var(--accent);border-color:var(--accent)}
  dl{display:grid;gap:6px;margin:10px 0;font:12px var(--mono)}
  dl div{display:flex;justify-content:space-between;gap:12px}
  dt{color:var(--fg-dim)}
  dd{margin:0;word-break:break-all}
  .row{font:12px var(--mono);padding:8px 0;border-top:1px solid var(--line);word-break:break-word}
  .empty{text-align:center;padding:16px}
</style>
