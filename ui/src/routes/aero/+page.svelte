<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  let status = $state<any>({});
  let messages = $state<any[]>([]);
  let busy = $state(false);
  let error = $state('');
  let enabled = $derived(Boolean(status.enabled ?? status.running ?? status.active));

  async function load() {
    try { [status, messages] = await Promise.all([Api.aeroStatus(), Api.aeroMessages()]); error = ''; }
    catch (e) { error = String(e); }
  }
  onMount(load);

  async function toggle() { busy = true; try { await Api.aeroEnable(!enabled); await load(); } catch (e) { error = String(e); } finally { busy = false; } }
  async function clear() { await Api.aeroClear(); await load(); }
</script>

<div class="page">
  <h1>Inmarsat AERO</h1><p class="muted">Decode and inspect Inmarsat aeronautical messages.</p>
  {#if error}<p class="error">{error}</p>{/if}
  <section class="card controls"><div><b>Status:</b> <span class:ok={enabled}>{enabled ? 'enabled' : 'disabled'}</span></div><button class="primary" onclick={toggle} disabled={busy}>{enabled ? 'Disable' : 'Enable'}</button><button onclick={clear}>Clear messages</button><button onclick={load}>Refresh</button></section>
  <section class="card"><h2>Messages</h2><div class="table">{#each messages as message}<div class="row">{#each Object.entries(message) as [key, value]}<span><small>{key}</small>{typeof value === 'object' ? JSON.stringify(value) : String(value ?? '—')}</span>{/each}</div>{:else}<div class="empty">No AERO messages</div>{/each}</div></section>
</div>

<style>
  .page{padding:16px;overflow-y:auto;height:100%}.muted,small,.empty{color:var(--fg-dim)}.error{color:var(--danger)}.card{margin:12px 0;padding:14px;background:var(--bg-elev);border:1px solid var(--line);border-radius:8px}.controls{display:flex;align-items:center;gap:10px;flex-wrap:wrap}.controls div{margin-right:auto}.ok{color:var(--ok)}button{background:var(--bg);color:var(--fg);border:1px solid var(--line);border-radius:5px;padding:7px 11px}.primary{border-color:var(--accent);color:var(--accent)}.row{display:flex;gap:16px;padding:8px 0;border-top:1px solid var(--line);font:12px var(--mono);flex-wrap:wrap}.row span{min-width:120px;word-break:break-word}.row small{display:block;font:10px var(--sans);text-transform:uppercase}h2{font-size:14px;margin:0 0 8px}.empty{padding:20px;text-align:center}
</style>
