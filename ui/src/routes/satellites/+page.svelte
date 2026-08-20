<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  let systems = $state<any[]>([]);
  let slices = $state<any[]>([]);
  let error = $state('');

  async function load() {
    try {
      const [gps, glonass, goes, sliceData] = await Promise.all([
        Api.gpsStatus(),
        Api.glonassStatus(),
        Api.goesStatus(),
        Api.protocolSlices(),
      ]);
      systems = [
        { id: 'gps', name: 'GPS', status: gps },
        { id: 'glonass', name: 'GLONASS', status: glonass },
        { id: 'goes', name: 'GOES LRIT', status: goes },
      ];
      slices = Array.isArray(sliceData?.slices) ? sliceData.slices : [];
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  onMount(load);

  function active(s: any) {
    return Boolean(s?.enabled ?? s?.running ?? s?.active);
  }

  async function toggle(system: any) {
    await Api.satelliteEnable(system.id, !active(system.status));
    await load();
  }

  async function clear(system: any) {
    await Api.satelliteClear(system.id);
    await load();
  }
</script>

<div class="page">
  <h1>Satellite Decoders</h1>
  <p class="muted">Combined GPS, GLONASS and GOES LRIT status plus planned protocol slices.</p>
  {#if error}<p class="error">{error}</p>{/if}

  <section class="slices">
    <h2>Protocol slices</h2>
    <p class="muted">Recorded-IQ fixtures and sidecar wiring remain required before decode is promoted.</p>
    <div class="slice-grid">
      {#each slices as slice}
        <article class:unavailable={!slice.available}>
          <b>{slice.name}</b>
          <small>{slice.id}</small>
          <p>{slice.description}</p>
          <span>{slice.available ? 'Fixture path declared' : slice.completion_reason ?? 'Unavailable until fixture is cleared'}</span>
          <code>{slice.fixture ?? 'no fixture'}</code>
        </article>
      {/each}
    </div>
  </section>

  <div class="grid">
    {#each systems as system}
      <section class="card">
        <h2>{system.name}</h2>
        <p class:ok={active(system.status)}>{active(system.status) ? 'Active' : 'Inactive'}</p>
        <div class="actions">
          <button class="primary" onclick={() => toggle(system)}>{active(system.status) ? 'Disable' : 'Enable'}</button>
          <button onclick={() => clear(system)}>Clear</button>
        </div>
        <pre>{JSON.stringify(system.status, null, 2)}</pre>
      </section>
    {/each}
  </div>
  <button onclick={load}>Refresh all</button>
</div>

<style>
  .page { padding: 16px; overflow-y: auto; height: 100%; max-width: 960px; }
  .muted { color: var(--fg-dim); }
  .error { color: var(--danger); }
  .slices { margin-bottom: 16px; }
  .slice-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 10px; margin-top: 10px; }
  .slice-grid article { padding: 12px; background: var(--bg-elev); border: 1px solid var(--line); border-radius: 8px; display: grid; gap: 4px; }
  .slice-grid article.unavailable { opacity: 0.85; }
  .slice-grid small, .slice-grid span { color: var(--fg-dim); font-size: 11px; }
  .slice-grid p { margin: 0; font-size: 12px; }
  .slice-grid code { font: 10px var(--mono); word-break: break-all; color: var(--accent-2); }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr)); gap: 12px; margin: 12px 0; }
  .card { padding: 14px; background: var(--bg-elev); border: 1px solid var(--line); border-radius: 8px; }
  .card h2 { margin: 0; }
  .ok { color: var(--ok); }
  .actions { display: flex; gap: 8px; }
  button { background: var(--bg); color: var(--fg); border: 1px solid var(--line); padding: 7px 11px; border-radius: 5px; }
  .primary { color: var(--accent); border-color: var(--accent); }
  pre { background: var(--bg); padding: 8px; border-radius: 5px; font: 11px var(--mono); overflow: auto; }
</style>
