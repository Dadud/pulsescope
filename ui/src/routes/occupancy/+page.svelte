<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  interface OccupancyCell {
    time_bucket_15min: number;
    frequency_bucket_hz: number;
    occupancy?: number;
    avg_power_db?: number;
  }

  interface HeatmapPayload {
    hours?: number;
    time_buckets?: number[];
    frequency_buckets_hz?: number[];
    cells?: OccupancyCell[];
  }

  let occupancy = $state<any[]>([]);
  let heatmap = $state<HeatmapPayload | null>(null);
  let heatmapHours = $state(24);
  let error = $state('');
  let canvas: HTMLCanvasElement;
  let rows = $derived(
    Array.isArray(occupancy) ? occupancy : [],
  );

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

  function occupancyColor(value: number) {
    const clamped = Math.max(0, Math.min(1, value));
    const hue = 210 - clamped * 190;
    const light = 18 + clamped * 42;
    return `hsl(${hue} 85% ${light}%)`;
  }

  function drawHeatmap(payload: HeatmapPayload | null) {
    if (!canvas || !payload?.cells?.length) return;
    const times = payload.time_buckets ?? [];
    const freqs = payload.frequency_buckets_hz ?? [];
    if (!times.length || !freqs.length) return;

    const dpr = window.devicePixelRatio || 1;
    const width = Math.max(canvas.clientWidth, 320);
    const height = Math.max(canvas.clientHeight, 220);
    canvas.width = Math.floor(width * dpr);
    canvas.height = Math.floor(height * dpr);
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = '#071018';
    ctx.fillRect(0, 0, width, height);

    const leftPad = 72;
    const bottomPad = 28;
    const topPad = 16;
    const plotW = width - leftPad - 12;
    const plotH = height - bottomPad - topPad;
    const cellW = plotW / times.length;
    const cellH = plotH / freqs.length;

    const lookup = new Map<string, number>();
    for (const cell of payload.cells) {
      lookup.set(`${cell.time_bucket_15min}:${cell.frequency_bucket_hz}`, Number(cell.occupancy ?? 0));
    }

    for (let ti = 0; ti < times.length; ti += 1) {
      for (let fi = 0; fi < freqs.length; fi += 1) {
        const value = lookup.get(`${times[ti]}:${freqs[fi]}`) ?? 0;
        ctx.fillStyle = occupancyColor(value);
        ctx.fillRect(leftPad + ti * cellW, topPad + (freqs.length - fi - 1) * cellH, cellW + 0.5, cellH + 0.5);
      }
    }

    ctx.strokeStyle = 'rgba(255,255,255,0.08)';
    ctx.strokeRect(leftPad, topPad, plotW, plotH);
    ctx.fillStyle = 'rgba(255,255,255,0.55)';
    ctx.font = '10px monospace';
    if (freqs.length) {
      ctx.fillText(fmtHz(freqs[freqs.length - 1]), 8, topPad + 10);
      ctx.fillText(fmtHz(freqs[0]), 8, topPad + plotH);
    }
    ctx.fillText(`${payload.hours ?? heatmapHours}h`, leftPad, height - 8);
    ctx.fillText('time →', leftPad + plotW - 42, height - 8);
  }

  async function load() {
    try {
      [occupancy, heatmap] = await Promise.all([
        Api.spectrumOccupancy(),
        Api.spectrumOccupancyHeatmap(heatmapHours),
      ]);
      error = '';
      drawHeatmap(heatmap);
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    void load();
    const timer = window.setInterval(() => void load(), 15000);
    const resize = () => drawHeatmap(heatmap);
    window.addEventListener('resize', resize);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener('resize', resize);
    };
  });

  $effect(() => {
    drawHeatmap(heatmap);
  });
</script>

<div class="page">
  <div class="heading">
    <div>
      <h1>Spectrum Occupancy</h1>
      <p class="muted">Live utilization meters plus a rolling frequency/time heatmap from 15-minute buckets.</p>
    </div>
    <div class="actions">
      <label>
        <span>Window</span>
        <select bind:value={heatmapHours} onchange={load}>
          <option value={6}>6 hours</option>
          <option value={12}>12 hours</option>
          <option value={24}>24 hours</option>
          <option value={48}>48 hours</option>
        </select>
      </label>
      <button onclick={load}>Refresh</button>
    </div>
  </div>
  {#if error}<p class="error">{error}</p>{/if}

  <section class="card heatmap-card">
    <div class="card-head">
      <h2>Band-use heatmap</h2>
      <p class="muted">Darker blue is quiet; warm colors show higher occupancy above the noise floor.</p>
    </div>
    <canvas bind:this={canvas} class="heatmap" aria-label="Spectrum occupancy heatmap"></canvas>
    {#if !heatmap?.cells?.length}
      <div class="empty">No heatmap history yet. Leave the receiver running to accumulate 15-minute buckets.</div>
    {/if}
  </section>

  <section class="card">
    <h2>Current window</h2>
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
  .page { padding: 16px; overflow-y: auto; height: 100%; }
  .heading { display: flex; justify-content: space-between; gap: 12px; align-items: flex-start; flex-wrap: wrap; }
  .actions { display: flex; gap: 8px; align-items: end; }
  .actions label { display: grid; gap: 4px; font-size: 11px; color: var(--fg-dim); }
  select, button { background: var(--bg); color: var(--fg); border: 1px solid var(--line); padding: 7px 11px; border-radius: 5px; }
  .muted, small, .empty { color: var(--fg-dim); }
  .error { color: var(--danger); }
  .card { margin: 12px 0; padding: 14px; background: var(--bg-elev); border: 1px solid var(--line); border-radius: 8px; }
  .card h2 { font-size: 14px; margin: 0 0 8px; }
  .card-head { margin-bottom: 10px; }
  .heatmap { width: 100%; height: 260px; display: block; border-radius: 6px; background: #071018; }
  .heatmap-card .empty { margin-top: 10px; text-align: center; }
  .row { display: grid; grid-template-columns: 1fr 180px 55px; gap: 12px; align-items: center; border-top: 1px solid var(--line); padding: 9px 0; font: 12px var(--mono); }
  .fields { display: flex; gap: 15px; flex-wrap: wrap; }
  .fields small { display: block; font: 10px var(--sans); }
  .meter { height: 8px; background: var(--bg); border-radius: 8px; overflow: hidden; }
  .meter i { display: block; height: 100%; background: var(--accent); }
  .empty { text-align: center; padding: 20px; }
  @media (max-width: 700px) {
    .row { grid-template-columns: 1fr; }
    .meter { width: 100%; }
  }
</style>
