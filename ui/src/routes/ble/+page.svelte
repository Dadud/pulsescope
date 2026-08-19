<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';
  let devices = $state<any[]>([]);
  let status = $state<any>({});
  let filter = $state('');
  let error = $state('');
  let shown = $derived(filter ? devices.filter((d) => JSON.stringify(d).toLowerCase().includes(filter.toLowerCase())) : devices);
  async function load() {
    try {
      [devices, status] = await Promise.all([Api.bleDevices(), Api.bleStatus()]);
      error = '';
    } catch (e) { error = String(e); }
  }
  onMount(load);
  async function clear() { await Api.bleClear(); await load(); }
</script>
<div class="page">
  <h1>Bluetooth LE</h1>
  <p class="muted">Advertising-channel GFSK decoder for channels 37/38/39. Encrypted payloads are identified only.</p>
  <div class="banner">Fixture-verified on synthesized IQ. RTL-SDR tuners cannot cover 2.4 GHz; use a 2.4 GHz-capable SDR.</div>
  {#if error}<p class="error">{error}</p>{/if}
  <section class="card controls">
    <b>{devices.length} devices</b>
    <input bind:value={filter} placeholder="Filter devices…" />
    <button onclick={load}>Refresh</button>
    <button class="danger" onclick={clear}>Clear</button>
    <pre>{JSON.stringify(status, null, 2)}</pre>
  </section>
  <section class="card">
    {#each shown as d}
      <div class="row">{#each Object.entries(d) as [k,v]}<span><small>{k}</small>{typeof v==='object'?JSON.stringify(v):String(v??'—')}</span>{/each}</div>
    {:else}
      <div class="empty">No BLE advertisements yet.</div>
    {/each}
  </section>
</div>
<style>
  .page{padding:16px;overflow-y:auto;height:100%}
  .muted,small,.empty{color:var(--fg-dim)}
  .error,.danger{color:var(--danger)}
  .banner{margin:8px 0 12px;padding:10px 12px;background:rgb(245 158 11 / 10%);border:1px solid rgb(245 158 11 / 30%);border-radius:7px;color:#fcd38d;font-size:12px}
  .card{margin:12px 0;padding:14px;background:var(--bg-elev);border:1px solid var(--line);border-radius:8px}
  .controls{display:flex;gap:9px;align-items:center;flex-wrap:wrap}
  .controls b{margin-right:auto}
  input,button{background:var(--bg);color:var(--fg);border:1px solid var(--line);padding:7px;border-radius:5px}
  pre{width:100%;font:11px var(--mono);background:var(--bg);padding:8px}
  .row{display:flex;gap:16px;border-top:1px solid var(--line);padding:8px 0;font:12px var(--mono);flex-wrap:wrap}
  .row span{min-width:120px;word-break:break-word}
  .row small{display:block;font:10px var(--sans)}
  .empty{text-align:center;padding:20px}
</style>
