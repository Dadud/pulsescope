<script lang="ts">
  import { onMount } from 'svelte';
  import { Api, type DecodedMessage, type VfoState } from '$lib/api';

  let vfos = $state<VfoState[]>([]);
  let messages = $state<DecodedMessage[]>([]);
  let signals = $state<any[]>([]);
  let health = $state<any>(null);
  let device = $state<any>(null);
  let jobs = $state<any[]>([]);
  let updatedAt = $state(0);
  let error = $state('');
  let loading = $state(true);

  function fmtHz(hz = 0) {
    if (hz >= 1e9) return `${(hz / 1e9).toFixed(6)} GHz`;
    if (hz >= 1e6) return `${(hz / 1e6).toFixed(4)} MHz`;
    return `${(hz / 1e3).toFixed(1)} kHz`;
  }
  function age(ms?: number) {
    if (!ms) return 'never';
    const seconds = Math.max(0, Math.round((Date.now() - ms) / 1000));
    return seconds < 60 ? `${seconds}s ago` : `${Math.floor(seconds / 60)}m ago`;
  }
  function stateLabel() {
    const capture = health?.capture ?? health?.components?.capture;
    return capture?.state ?? health?.status ?? device?.state ?? (device?.connected ? 'ready' : 'offline');
  }
  async function refresh() {
    const results = await Promise.allSettled([
      Api.vfoStates(), Api.decodedMessages(30), Api.signalEvents(30),
      Api.systemHealthV2(), Api.deviceStatus(), Api.decoderJobsV2()
    ]);
    if (results[0].status === 'fulfilled') vfos = results[0].value;
    if (results[1].status === 'fulfilled') messages = results[1].value;
    if (results[2].status === 'fulfilled') signals = results[2].value;
    if (results[3].status === 'fulfilled') health = results[3].value;
    if (results[4].status === 'fulfilled') device = results[4].value;
    if (results[5].status === 'fulfilled') {
      const value: any = results[5].value;
      jobs = value?.jobs ?? value?.decoder_jobs ?? (Array.isArray(value) ? value : []);
    }
    const failures = results.filter((r) => r.status === 'rejected');
    error = failures.length === results.length ? 'Receiver API is unavailable. Retrying automatically.' : '';
    updatedAt = Date.now();
    loading = false;
  }
  async function listen(vfo: VfoState) {
    await Api.vfoMute(vfo.id, !vfo.muted);
    await refresh();
  }
  onMount(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 1500);
    const visible = () => { if (!document.hidden) void refresh(); };
    document.addEventListener('visibilitychange', visible);
    return () => { window.clearInterval(timer); document.removeEventListener('visibilitychange', visible); };
  });
</script>

<svelte:head><title>Monitor · PulseScope</title></svelte:head>

<section class="monitor-page">
  <header class="page-head">
    <div><p class="eyebrow">Shared receiver overview</p><h1>Monitor</h1><p>Watch active VFOs, decoder traffic, and receiver health without crowding the tuning workspace.</p></div>
    <div class:bad={error} class="freshness"><span class="dot"></span>{error || `Updated ${age(updatedAt)}`}</div>
    <a class="device-link" href="#/settings">Device setup</a>
  </header>

  <div class="summary-grid">
    <article class="card summary"><span>Receiver</span><strong>{stateLabel()}</strong><small>{device?.label ?? device?.name ?? device?.driver ?? 'No hardware label'}</small></article>
    <article class="card summary"><span>Active VFOs</span><strong>{vfos.length}</strong><small>{vfos.filter((v) => !v.muted).length} listening</small></article>
    <article class="card summary"><span>Recent signals</span><strong>{signals.length}</strong><small>latest {age(signals[0]?.timestamp_ms)}</small></article>
    <article class="card summary"><span>Decoder processes</span><strong>{jobs.length}</strong><small>{jobs.filter((j) => ['running','ready','active'].includes(String(j.status ?? j.state).toLowerCase())).length} active · scheduled jobs are under More</small></article>
  </div>

  {#if error}<div class="callout" role="status">{error}</div>{/if}

  <div class="workspace-grid">
    <section class="card panel vfo-panel">
      <div class="panel-head"><div><span class="eyebrow">Live channels</span><h2>VFOs</h2></div><a href="#/">Tune receiver →</a></div>
      <div class="vfo-grid">
        {#each vfos as vfo (vfo.id)}
          <article class:active={!vfo.muted} class="vfo-card">
            <div class="vfo-title"><span>VFO {vfo.id + 1}</span><b>{vfo.mode}</b></div>
            <strong class="frequency">{fmtHz(vfo.frequency_hz)}</strong>
            <div class="meter" aria-label={`Signal ${vfo.strength_db.toFixed(0)} dB`}><i style={`width:${Math.max(2, Math.min(100, vfo.strength_db + 110))}%`}></i></div>
            <div class="vfo-meta"><span>{vfo.strength_db.toFixed(0)} dB</span><span>{vfo.squelch_open ? 'Squelch open' : 'Quiet'}</span></div>
            <button class:primary={vfo.muted} onclick={() => listen(vfo)}>{vfo.muted ? 'Listen' : 'Stop listening'}</button>
          </article>
        {:else}
          <div class="empty">{loading ? 'Loading VFOs…' : 'No VFO is allocated. Open Receiver to create one.'}</div>
        {/each}
      </div>
    </section>

    <section class="card panel activity-panel">
      <div class="panel-head"><div><span class="eyebrow">Decoded traffic</span><h2>Latest activity</h2></div><a href="#/messages">All activity →</a></div>
      <div class="activity-list">
        {#each messages.slice(0, 12) as message (message.id ?? `${message.timestamp_ms}-${message.frequency_hz}`)}
          <article><div><b>{message.protocol || 'Unknown'}</b><span>{fmtHz(message.frequency_hz)}</span></div><p>{message.content || message.message_type || 'Decoder event'}</p><time>{age(message.timestamp_ms)}</time></article>
        {:else}<div class="empty">No decoded messages yet.</div>{/each}
      </div>
    </section>
  </div>
</section>

<style>
  .monitor-page{height:100%;overflow:auto;padding:clamp(12px,2vw,24px);max-width:1600px;margin:auto}.page-head{display:flex;justify-content:space-between;align-items:end;gap:24px;margin-bottom:18px}.page-head h1{font-size:30px;margin:2px 0}.page-head p{margin:0;color:var(--fg-dim);max-width:720px}.eyebrow{text-transform:uppercase;letter-spacing:.11em;font-size:10px;color:var(--accent)!important;font-weight:700}.freshness{display:flex;align-items:center;gap:7px;color:var(--fg-dim);font-size:12px;white-space:nowrap}.device-link{color:var(--accent-2);text-decoration:none;font-size:12px;white-space:nowrap;padding:6px 10px;border:1px solid var(--line-strong);border-radius:6px}.device-link:hover{border-color:var(--accent)}.dot{width:8px;height:8px;border-radius:50%;background:var(--ok);box-shadow:0 0 8px var(--ok)}.freshness.bad .dot{background:var(--danger);box-shadow:none}.summary-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px;margin-bottom:12px}.summary{display:grid;gap:3px}.summary span,.summary small{color:var(--fg-dim);font-size:11px}.summary strong{font-size:22px;text-transform:capitalize}.callout{padding:10px 12px;border:1px solid var(--danger);border-radius:7px;color:#fecdd3;margin-bottom:12px}.workspace-grid{display:grid;grid-template-columns:minmax(0,1.7fr) minmax(320px,1fr);gap:12px}.panel{padding:14px}.panel-head{display:flex;align-items:end;justify-content:space-between;margin-bottom:12px}.panel-head h2{margin:2px 0;font-size:17px;color:var(--fg);text-transform:none;letter-spacing:0}.panel-head a{color:var(--accent-2);text-decoration:none;font-size:12px}.vfo-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(210px,1fr));gap:9px}.vfo-card{border:1px solid var(--line);border-radius:8px;padding:12px;background:var(--bg);display:grid;gap:9px}.vfo-card.active{border-color:var(--accent)}.vfo-title,.vfo-meta{display:flex;justify-content:space-between;color:var(--fg-dim);font-size:11px}.vfo-title b{color:var(--accent-2)}.frequency{font:600 18px var(--mono)}.meter{height:7px;background:var(--bg-elev-2);border-radius:5px;overflow:hidden}.meter i{display:block;height:100%;background:linear-gradient(90deg,var(--accent-2),var(--accent),var(--warn))}.activity-list{display:grid}.activity-list article{display:grid;grid-template-columns:1fr auto;gap:2px 12px;padding:9px 0;border-top:1px solid var(--line)}.activity-list article>div{display:flex;gap:8px;font-size:11px}.activity-list article div span,.activity-list time{color:var(--fg-dim);font-size:10px}.activity-list p{grid-column:1;margin:0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.activity-list time{grid-column:2;grid-row:1/3}.empty{padding:24px;text-align:center;color:var(--fg-dim)}
  @media(max-width:900px){.summary-grid{grid-template-columns:repeat(2,1fr)}.workspace-grid{grid-template-columns:1fr}.page-head{align-items:start;flex-direction:column;gap:10px}.device-link{align-self:stretch;text-align:center;min-height:44px;display:flex;align-items:center;justify-content:center}}
  @media(max-width:520px){.monitor-page{padding:10px}.page-head h1{font-size:24px}.summary-grid{gap:6px}.summary{padding:9px}.summary strong{font-size:18px}.vfo-grid{grid-template-columns:1fr}.activity-list article{grid-template-columns:1fr}.activity-list time{grid-column:1;grid-row:auto}}
</style>
