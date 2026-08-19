<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  let status = $state<any>({});
  let calls = $state<any[]>([]);
  let discoveries = $state<any[]>([]);
  let error = $state('');
  let busy = $state(false);

  async function load() {
    try {
      [status, calls, discoveries] = await Promise.all([
        Api.trunkingStatus(), Api.trunkingCalls(), Api.trunkingDiscoveryResults()
      ]);
      error = '';
    } catch (e) { error = String(e); }
  }
  onMount(load);

  async function toggle() {
    busy = true;
    try { if (status.running) await Api.trunkingStop(); else await Api.trunkingStart(); await load(); }
    catch (e) { error = String(e); }
    finally { busy = false; }
  }
  async function lock() {
    try { await Api.trunkingLock(!status.locked); await load(); } catch (e) { error = String(e); }
  }
  async function discover() {
    try {
      if (status.discovery_running) await Api.trunkingDiscoveryStop();
      else await Api.trunkingDiscoveryStart();
      await load();
    } catch (e) { error = String(e); }
  }
</script>

<div class="trunking-page">
  <h1>Trunking</h1>
  <p class="muted">Follow P25 talkgroup grants from a control-channel TSBK observer. Encrypted calls are labeled only. NXDN/EDACS/DMR controllers are not claimed.</p>
  <div class="banner">Beta: P25 FIR + TSBK parse is implemented. Live trunking stays unavailable until a control-channel recorded-IQ / hardware gate passes. No mock talkgroups are invented.</div>
  {#if error}<p class="error">{error}</p>{/if}

  <section class="card controls">
    <button class="primary" disabled={busy} onclick={toggle}>{status.running ? 'Stop' : 'Start'} trunking</button>
    <button onclick={lock}>{status.locked ? 'Unlock system' : 'Lock system'}</button>
    <button onclick={discover}>{status.discovery_running ? 'Stop discovery' : 'Discover systems'}</button>
    <button onclick={load}>Refresh</button>
    <span class:online={status.running} class="state">{status.running ? 'RUNNING' : 'IDLE'}</span>
  </section>

  <section class="grid">
    <div class="card"><h2>System</h2><dl>
      <dt>Name</dt><dd>{status.system ?? '—'}</dd>
      <dt>Protocol</dt><dd>{status.protocol ?? 'P25 / auto'}</dd>
      <dt>Control channel</dt><dd>{status.control_channel_hz ?? '—'}</dd>
      <dt>Active talkgroup</dt><dd>{status.active_talkgroup ?? '—'}</dd>
    </dl></div>
    <div class="card"><h2>Discovery</h2>{#if discoveries.length}<ul>{#each discoveries as item}<li>{JSON.stringify(item)}</li>{/each}</ul>{:else}<p class="muted">No discovered systems.</p>{/if}</div>
  </section>

  <section class="card"><h2>Call history</h2>
    {#if calls.length}<div class="calls">{#each calls as call}<div class="call"><b>{call.talkgroup ?? call.talkgroup_id ?? 'Unknown TG'}</b><span>{call.frequency_hz ?? '—'}</span><span>{call.started_ms ?? call.timestamp_ms ?? '—'}</span></div>{/each}</div>{:else}<p class="muted">No trunking calls recorded.</p>{/if}
  </section>
</div>

<style>
  .trunking-page { padding:16px; overflow-y:auto; height:100%; }
  .muted { color:var(--fg-dim); }.error { color:var(--danger); }
  .banner { margin:8px 0 12px; padding:10px 12px; background:rgb(245 158 11 / 10%); border:1px solid rgb(245 158 11 / 30%); border-radius:7px; color:#fcd38d; font-size:12px; }
  .card { margin:12px 0; padding:14px; background:var(--bg-elev); border:1px solid var(--line); border-radius:8px; }
  .controls { display:flex; gap:8px; align-items:center; flex-wrap:wrap; }
  button { background:var(--bg); color:var(--fg); border:1px solid var(--line); padding:7px 10px; border-radius:5px; cursor:pointer; }
  button.primary { color:var(--accent); border-color:var(--accent); } button:disabled { opacity:.5; }
  .state { margin-left:auto; color:var(--fg-dim); font:11px var(--mono); }.state.online { color:var(--ok); }
  .grid { display:grid; grid-template-columns:1fr 1fr; gap:12px; } h2 { font-size:14px; margin-top:0; }
  dl { display:grid; grid-template-columns:150px 1fr; gap:6px; font:12px var(--mono); } dt { color:var(--fg-dim); } dd { margin:0; }
  ul { padding-left:18px; font:11px var(--mono); }.call { display:grid; grid-template-columns:1fr 1fr 1fr; gap:10px; padding:7px 0; border-bottom:1px solid var(--line); font:12px var(--mono); }
  @media (max-width:700px) { .grid { grid-template-columns:1fr; } }
</style>
