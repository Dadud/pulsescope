<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';
  let status = $state<any>({}); let messages = $state<any[]>([]); let error = $state('');
  let enabled = $derived(Boolean(status.enabled ?? status.running ?? status.active));
  async function load(){try{[status,messages]=await Promise.all([Api.iridiumStatus(),Api.iridiumMessages()]);error='';}catch(e){error=String(e)}}
  onMount(load);
  async function toggle(){await Api.iridiumEnable(!enabled);await load()}
  async function quickStart(){await Api.iridiumQuickStart();await load()}
  async function clear(){await Api.iridiumClear();await load()}
</script>
<div class="page"><h1>Iridium</h1><p class="muted">Monitor Iridium satellite downlink traffic.</p>{#if error}<p class="error">{error}</p>{/if}
<section class="card controls"><b>Status: <span class:ok={enabled}>{enabled?'enabled':'disabled'}</span></b><button class="primary" onclick={toggle}>{enabled?'Disable':'Enable'}</button><button onclick={quickStart}>Quick start</button><button onclick={clear}>Clear</button><button onclick={load}>Refresh</button></section>
<section class="card"><h2>Messages</h2>{#each messages as m}<div class="row">{#each Object.entries(m) as [k,v]}<span><small>{k}</small>{typeof v==='object'?JSON.stringify(v):String(v??'—')}</span>{/each}</div>{:else}<div class="empty">No Iridium messages</div>{/each}</section></div>
<style>.page{padding:16px;overflow-y:auto;height:100%}.muted,small,.empty{color:var(--fg-dim)}.error{color:var(--danger)}.card{margin:12px 0;padding:14px;background:var(--bg-elev);border:1px solid var(--line);border-radius:8px}.controls{display:flex;gap:10px;align-items:center;flex-wrap:wrap}.controls b{margin-right:auto}.ok{color:var(--ok)}button{background:var(--bg);color:var(--fg);border:1px solid var(--line);padding:7px 11px;border-radius:5px}.primary{color:var(--accent);border-color:var(--accent)}h2{font-size:14px;margin:0 0 8px}.row{display:flex;gap:16px;border-top:1px solid var(--line);padding:8px 0;font:12px var(--mono);flex-wrap:wrap}.row span{min-width:120px;word-break:break-word}.row small{display:block;font:10px var(--sans);text-transform:uppercase}.empty{text-align:center;padding:20px}</style>
