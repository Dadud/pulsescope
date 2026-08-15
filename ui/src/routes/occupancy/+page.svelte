<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  let occupancy = $state<any>([]);
  let error = $state('');
  let rows = $derived(Array.isArray(occupancy) ? occupancy : (occupancy?.bins ?? occupancy?.entries ?? []));

  function fmtHz(hz: number) {
    if (hz >= 1e6) return `${(hz / 1e6).toFixed(4)} MHz`;
    if (hz >= 1e3) return `${(hz / 1e3).toFixed(1)} kHz`;
    return `${hz} Hz`;
  }
  function pct(row: any) {
    const n = Number(row?.occupancy ?? row?.percent ?? row?.utilization);
    if (!Number.isFinite(n)) return 0;
    return n <= 1 ? n * 100 : n;
  }

  async function load() {
    try {
      occupancy = await Api.spectrumOccupancy();
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    void load();
    const timer = window.setInterval(() => void load(), 5000);
    return () => window.clearInterval(timer);
  });
</script>

<div class="page">
  <div class="heading">
    <div>
      <h1>Spectrum Occupancy</h1>
      <p class="muted">Utilization of the current capture window, recorded while the receiver is running.</p>
    </div>
    <button onclick={load}>Refresh</button>
  </div>
  {#if error}<p class="error">{error}</p>{/if}
  <section class="card">
    {#each rows as row}
      <div class="row">
        <div class="fields">
          <span><small>Frequency</small>{fmtHz(Number(row.frequency_bucket_hz ?? 0))}</span>
          <span><small>Average</small>{Number(row.avg_power_db ?? 0).toFixed(1)} dBFS</span>
          <span><small>Peak</small>{Number(row.peak_power_db ?? 0).toFixed(1)} dBFS</span>
          <span><small>Above floor</small>{Number(row.avg_above_floor_db ?? 0).toFixed(1)} dB</span>
        </div>
        <div class="meter"><i style={`width:${Math.min(100, pct(row))}%`}></i></div>
        <b>{pct(row).toFixed(1)}%</b>
      </div>
    {:else}
      <div class="empty">No occupancy samples yet. Start the receiver and wait a few seconds.</div>
    {/each}
  </section>
</div>
<style>
  .page{padding:16px;overflow-y:auto;height:100%}
  .heading{display:flex;justify-content:space-between;align-items:center}
  .muted,small,.empty{color:var(--fg-dim)}
  .error{color:var(--danger)}
  button{background:var(--bg);color:var(--fg);border:1px solid var(--line);padding:7px 11px;border-radius:5px}
  .card{margin:12px 0;padding:14px;background:var(--bg-elev);border:1px solid var(--line);border-radius:8px}
  .row{display:grid;grid-template-columns:1fr 180px 55px;gap:12px;align-items:center;border-top:1px solid var(--line);padding:9px 0;font:12px var(--mono)}
  .fields{display:flex;gap:15px;flex-wrap:wrap}
  .fields small{display:block;font:10px var(--sans)}
  .meter{height:8px;background:var(--bg);border-radius:8px;overflow:hidden}
  .meter i{display:block;height:100%;background:var(--accent)}
  .empty{text-align:center;padding:20px}
  @media(max-width:700px){.row{grid-template-columns:1fr}.meter{width:100%}}
</style>
