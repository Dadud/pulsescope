<script lang="ts">
  import { onMount } from 'svelte';
  import { Api, type ScanRange } from '$lib/api';
  let jobs: any[] = $state([]), banks: ScanRange[] = $state([]), range = $state('FM Broadcast'), delay = $state(1), duration = $state(60), notice = $state('');
  async function load(){ try { jobs=(await Api.jobs()).jobs; banks=await Api.banks(); if(!banks.some(b=>b.name===range)) range=banks[0]?.name??''; }catch(e){notice=String(e)} }
  async function create(){ try { await Api.createJob({name:`${range} scan`,kind:'scan',payload:{range_name:range,duration_ms:Math.max(1,duration)*1000},next_run_ms:Date.now()+Math.max(0,delay)*1000}); await load(); notice='Scheduled.' }catch(e){notice=String(e)} }
  async function remove(id:number){ await Api.deleteJob(id); await load(); }
  onMount(()=>{void load()});
</script>
<section class="page"><h1>Jobs</h1><p>One-shot scheduled scans claim the receiver, run muted, and release it at completion.</p>
<div class="card form"><label>Range <select bind:value={range}>{#each banks as b}<option value={b.name}>{b.name}</option>{/each}</select></label><label>Start in seconds <input type="number" min="0" bind:value={delay}/></label><label>Duration seconds <input type="number" min="1" max="3600" bind:value={duration}/></label><button onclick={create} disabled={!range}>Schedule scan</button></div>
{#if notice}<p class="notice">{notice}</p>{/if}
<div class="card"><h2>Scheduled jobs</h2>{#each jobs as j}<div class="job"><b>{j.name}</b><span>{j.kind} · {j.last_status}</span><span>{j.next_run_ms ? new Date(j.next_run_ms).toLocaleString() : 'not queued'}</span><button onclick={()=>remove(j.id)}>Delete</button></div>{:else}<p>No jobs.</p>{/each}</div></section>
<style>.page{padding:16px;max-width:900px}.card{padding:14px;margin:12px 0}.form{display:flex;gap:12px;align-items:end;flex-wrap:wrap}.form label{display:grid;gap:4px;font-size:12px}.job{display:grid;grid-template-columns:1fr 140px 220px auto;gap:10px;padding:9px 0;border-top:1px solid var(--line);font-size:12px}.notice{color:var(--ok)}</style>
