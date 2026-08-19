<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';
  let messages = $state<any[]>([]);
  let regions = $state<any[]>([]);
  let scan = $state<any>({});
  let error = $state('');
  async function load() {
    try {
      [messages, regions] = await Promise.all([Api.loraMessages(), Api.loraRegions()]);
      error = '';
    } catch (e) { error = String(e); }
  }
  async function scanNow() {
    try { scan = await Api.scanLora(); await load(); }
    catch (e) { error = String(e); }
  }
  onMount(load);
</script>
<div class="page">
  <h1>LoRa</h1>
  <p class="muted">Native CSS PHY plus MeshCore, Meshtastic, Reticulum, Modbus RTU, and LoRaWAN identification. Encrypted bodies stay opaque.</p>
  <div class="banner">Beta: fixture-verified on synthesized IQ. Live ISM reception still needs a matching sample rate and hardware evidence.</div>
  {#if error}<p class="error">{error}</p>{/if}
  <section class="card">
    <div class="title"><h2>Regional plans</h2><div><button onclick={scanNow}>Scan snapshot</button><button onclick={load}>Refresh</button></div></div>
    <p class="muted">These are documented band plans, not discovered radios.</p>
    <div class="chips">{#each regions as r}<span>{typeof r === 'object' ? (r.id ?? JSON.stringify(r)) : r}</span>{:else}<i>No regions reported</i>{/each}</div>
    {#if scan?.note || scan?.reason}<p class="muted">{scan.note ?? scan.reason}</p>{/if}
  </section>
  <section class="card">
    <h2>Messages</h2>
    {#each messages as m}
      <div class="row">
        <span><small>family</small>{m.protocol ?? '—'}</span>
        <span><small>type</small>{m.message_type ?? '—'}</span>
        <span><small>from</small>{m.address ?? '—'}</span>
        <span><small>encryption</small>{m.encryption ?? '—'}</span>
        <span><small>content</small>{m.content ?? '—'}</span>
      </div>
    {:else}
      <div class="empty">No LoRa messages yet. Tune an ISM channel and scan a snapshot.</div>
    {/each}
  </section>
</div>
<style>
  .page{padding:16px;overflow-y:auto;height:100%}
  .muted,small,.empty,i{color:var(--fg-dim)}
  .error{color:var(--danger)}
  .banner{margin:8px 0 12px;padding:10px 12px;background:rgb(245 158 11 / 10%);border:1px solid rgb(245 158 11 / 30%);border-radius:7px;color:#fcd38d;font-size:12px}
  .card{margin:12px 0;padding:14px;background:var(--bg-elev);border:1px solid var(--line);border-radius:8px}
  .title{display:flex;justify-content:space-between;gap:8px;align-items:center}
  .title h2,h2{margin:0 0 8px;font-size:14px}
  button,.chips span{background:var(--bg);color:var(--fg);border:1px solid var(--line);padding:6px 9px;border-radius:5px}
  .title div{display:flex;gap:8px}
  .chips{display:flex;gap:7px;flex-wrap:wrap;font:12px var(--mono)}
  .row{display:flex;gap:16px;border-top:1px solid var(--line);padding:8px 0;font:12px var(--mono);flex-wrap:wrap}
  .row span{min-width:120px}
  .row small{display:block;font:10px var(--sans)}
  .empty{text-align:center;padding:20px}
</style>
