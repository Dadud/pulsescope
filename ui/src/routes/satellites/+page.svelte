<script lang="ts">
  import { onMount } from 'svelte'; import { Api } from '$lib/api';
  let systems = $state<any[]>([]); let error=$state('');
  async function load(){try{const [gps,glonass,goes]=await Promise.all([Api.gpsStatus(),Api.glonassStatus(),Api.goesStatus()]);systems=[{id:'gps',name:'GPS',status:gps},{id:'glonass',name:'GLONASS',status:glonass},{id:'goes',name:'GOES LRIT',status:goes}];error=''}catch(e){error=String(e)}}
  onMount(load);
  function active(s:any){return Boolean(s?.enabled??s?.running??s?.active)}
  async function toggle(system:any){await Api.satelliteEnable(system.id,!active(system.status));await load()}
  async function clear(system:any){await Api.satelliteClear(system.id);await load()}
</script>
<div class="page"><h1>Satellite Decoders</h1><p class="muted">Combined GPS, GLONASS and GOES LRIT status.</p>{#if error}<p class="error">{error}</p>{/if}<div class="grid">{#each systems as system}<section class="card"><h2>{system.name}</h2><p class:ok={active(system.status)}>{active(system.status)?'Active':'Inactive'}</p><div class="actions"><button class="primary" onclick={()=>toggle(system)}>{active(system.status)?'Disable':'Enable'}</button><button onclick={()=>clear(system)}>Clear</button></div><pre>{JSON.stringify(system.status,null,2)}</pre></section>{/each}</div><button onclick={load}>Refresh all</button></div>
<style>.page{padding:16px;overflow-y:auto;height:100%}.muted{color:var(--fg-dim)}.error{color:var(--danger)}.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(250px,1fr));gap:12px;margin:12px 0}.card{padding:14px;background:var(--bg-elev);border:1px solid var(--line);border-radius:8px}.card h2{margin:0}.ok{color:var(--ok)}.actions{display:flex;gap:8px}button{background:var(--bg);color:var(--fg);border:1px solid var(--line);padding:7px 11px;border-radius:5px}.primary{color:var(--accent);border-color:var(--accent)}pre{background:var(--bg);padding:8px;border-radius:5px;font:11px var(--mono);overflow:auto}</style>
