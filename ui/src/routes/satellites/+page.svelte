<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  type Slice = {
    id: string;
    name: string;
    description?: string;
    available?: boolean;
    completion_reason?: string;
    fixture?: string;
    transport?: string;
    ui_outcome?: string;
    message_schema?: string;
  };

  type Product = {
    satellite: string;
    product: string;
    channel: string;
    timestamp_ms: number;
    file_path: string;
    valid: boolean;
  };

  type Telemetry = {
    address: string;
    content: string;
    timestamp_ms: number;
    frequency_hz: number;
    function_code?: string;
  };

  let slices = $state<Slice[]>([]);
  let goes = $state<any>({});
  let sonde = $state<any>({});
  let products = $state<Product[]>([]);
  let telemetry = $state<Telemetry[]>([]);
  let gps = $state<any>({});
  let glonass = $state<any>({});
  let error = $state('');
  let snapshotNote = $state('');

  function field(content: string, key: string): string {
    const match = content.match(new RegExp(`${key}=([^\\s]+)`));
    return match?.[1] ?? '';
  }

  function validSondes(rows: Telemetry[]) {
    return rows.filter((row) => field(row.content, 'checksum_valid') === 'true');
  }

  async function load() {
    try {
      const [sliceData, goesStatus, goesProducts, sondeStatus, sondeRows, gpsStatus, glonassStatus] =
        await Promise.all([
          Api.protocolSlices(),
          Api.goesStatus(),
          Api.goesProducts(),
          Api.radiosondeStatus(),
          Api.radiosondeTelemetry(),
          Api.gpsStatus(),
          Api.glonassStatus(),
        ]);
      slices = Array.isArray(sliceData?.slices) ? sliceData.slices : [];
      goes = goesStatus ?? {};
      products = Array.isArray(goesProducts?.products) ? goesProducts.products : [];
      sonde = sondeStatus ?? {};
      telemetry = Array.isArray(sondeRows) ? sondeRows : [];
      gps = gpsStatus ?? {};
      glonass = glonassStatus ?? {};
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  onMount(load);

  async function toggleGoes() {
    await Api.satelliteEnable('goes', !Boolean(goes.enabled));
    await load();
  }

  async function toggleSonde() {
    await Api.radiosondeEnable(!Boolean(sonde.enabled));
    await load();
  }

  async function snapshot(kind: 'goes' | 'radiosonde') {
    snapshotNote = kind === 'goes' ? JSON.stringify(await Api.scanGoes()) : JSON.stringify(await Api.scanRadiosonde());
    await load();
  }

  const plotted = $derived(
    validSondes(telemetry).map((row) => {
      const lat = Number(field(row.content, 'lat'));
      const lon = Number(field(row.content, 'lon'));
      return { row, lat, lon, x: ((lon + 180) / 360) * 100, y: ((90 - lat) / 180) * 100 };
    }).filter((p) => Number.isFinite(p.lat) && Number.isFinite(p.lon))
  );
</script>

<div class="page">
  <h1>Satellites and radiosondes</h1>
  <p class="muted">GOES product identification and radiosonde telemetry from recorded-IQ native decoders. Image reconstruction and live Vaisala FEC stay on signed sidecars.</p>
  {#if error}<p class="error">{error}</p>{/if}

  <section class="slices">
    <h2>Protocol slices</h2>
    <div class="slice-grid">
      {#each slices as slice}
        <article class:unavailable={!slice.available}>
          <b>{slice.name}</b>
          <small>{slice.id}</small>
          <p>{slice.description}</p>
          <span>{slice.available ? 'Fixture verified' : slice.completion_reason ?? 'Unavailable until fixture is cleared'}</span>
          <code>{slice.fixture ?? 'no fixture'}</code>
          {#if slice.ui_outcome}<p>{slice.ui_outcome}</p>{/if}
        </article>
      {/each}
    </div>
  </section>

  <div class="grid">
    <section class="card">
      <h2>GOES LRIT/HRIT</h2>
      <p class:ok={goes.available}>Native CADU identification {goes.available ? 'available' : 'unavailable'}</p>
      <p class="muted">SatDump image pipeline: {goes.image_pipeline ? 'running' : 'not claimed'}. Configured output paths are never listed as reception.</p>
      <div class="actions">
        <button class="primary" onclick={toggleGoes}>{goes.enabled ? 'Disable' : 'Enable'}</button>
        <button onclick={() => snapshot('goes')}>Snapshot live IQ</button>
        <button onclick={async () => { await Api.satelliteClear('goes'); await load(); }}>Clear products</button>
      </div>
      {#if products.length}
        <div class="gallery">
          {#each products.filter((p) => p.valid) as product}
            <article>
              <b>{product.product}</b>
              <span>{product.satellite} ch {product.channel || '—'}</span>
              <small>{product.file_path || 'inline'} · {new Date(product.timestamp_ms).toLocaleString()}</small>
            </article>
          {/each}
        </div>
      {:else}
        <div class="empty">No checksum-valid GOES products yet.</div>
      {/if}
      <pre>{JSON.stringify({ enabled: goes.enabled, native: goes.native, image_pipeline: goes.image_pipeline, satdump: goes.satdump, products_from_output_dir: goes.products_from_output_dir }, null, 2)}</pre>
    </section>

    <section class="card">
      <h2>Radiosondes</h2>
      <p class:ok={sonde.available}>Native GFSK telemetry {sonde.available ? 'available' : 'unavailable'}</p>
      <p class="muted">Map and table show checksum-valid frames only. Live Vaisala RS41 FEC uses rs41mod when installed.</p>
      <div class="actions">
        <button class="primary" onclick={toggleSonde}>{sonde.enabled ? 'Disable' : 'Enable'}</button>
        <button onclick={() => snapshot('radiosonde')}>Snapshot live IQ</button>
        <button onclick={async () => { await Api.radiosondeClear(); await load(); }}>Clear telemetry</button>
      </div>
      <div class="map" aria-label="Radiosonde positions">
        {#each plotted as point}
          <span class="dot" style="left:{point.x}%;top:{point.y}%" title="{point.row.address} {point.lat.toFixed(3)},{point.lon.toFixed(3)}"></span>
        {/each}
        {#if !plotted.length}
          <div class="empty">No checksum-valid positions yet.</div>
        {/if}
      </div>
      <div class="table">
        {#each validSondes(telemetry) as row}
          <div class="row">
            <b>{row.address || '—'}</b>
            <span>{row.function_code || 'RS41'}</span>
            <span>{field(row.content, 'lat')} / {field(row.content, 'lon')}</span>
            <span>{field(row.content, 'altitude_m')} m</span>
            <span>{field(row.content, 'temperature_c')} °C</span>
          </div>
        {:else}
          <div class="empty">No checksum-valid radiosonde frames.</div>
        {/each}
      </div>
      <pre>{JSON.stringify({ enabled: sonde.enabled, native: sonde.native, rs41mod: sonde.rs41mod, live_vaisala_sidecar: sonde.live_vaisala_sidecar }, null, 2)}</pre>
    </section>
  </div>

  <section class="card">
    <h2>GPS / GLONASS acquisition</h2>
    <p class="muted">Acquisition slices remain unavailable until recorded-IQ fixtures pass. Enable flags persist configuration only.</p>
    <pre>{JSON.stringify({ gps, glonass }, null, 2)}</pre>
  </section>

  {#if snapshotNote}<pre class="snap">{snapshotNote}</pre>{/if}
  <button onclick={load}>Refresh all</button>
</div>

<style>
  .page { padding: 16px; overflow-y: auto; height: 100%; max-width: 1100px; }
  .muted { color: var(--fg-dim); }
  .error { color: var(--danger); }
  .slices { margin-bottom: 16px; }
  .slice-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 10px; margin-top: 10px; }
  .slice-grid article { padding: 12px; background: var(--bg-elev); border: 1px solid var(--line); border-radius: 8px; display: grid; gap: 4px; }
  .slice-grid article.unavailable { opacity: 0.85; }
  .slice-grid small, .slice-grid span { color: var(--fg-dim); font-size: 11px; }
  .slice-grid p { margin: 0; font-size: 12px; }
  .slice-grid code { font: 10px var(--mono); word-break: break-all; color: var(--accent-2); }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 12px; margin: 12px 0; }
  .card { padding: 14px; background: var(--bg-elev); border: 1px solid var(--line); border-radius: 8px; }
  .card h2 { margin: 0 0 6px; }
  .ok { color: var(--ok); }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; margin: 8px 0; }
  button { background: var(--bg); color: var(--fg); border: 1px solid var(--line); padding: 7px 11px; border-radius: 5px; }
  .primary { color: var(--accent); border-color: var(--accent); }
  pre { background: var(--bg); padding: 8px; border-radius: 5px; font: 11px var(--mono); overflow: auto; }
  .empty { color: var(--fg-dim); text-align: center; padding: 16px; }
  .gallery { display: grid; gap: 8px; margin: 8px 0; }
  .gallery article { padding: 8px; border: 1px solid var(--line); border-radius: 6px; display: grid; }
  .gallery small { color: var(--fg-dim); }
  .map { position: relative; height: 180px; background: #071018; border-radius: 6px; overflow: hidden; margin: 8px 0; }
  .dot { position: absolute; width: 8px; height: 8px; margin: -4px 0 0 -4px; border-radius: 50%; background: var(--accent); }
  .table { display: grid; }
  .row { display: grid; grid-template-columns: 90px 90px 1fr 80px 80px; gap: 8px; font: 12px var(--mono); padding: 6px 0; border-top: 1px solid var(--line); }
  .snap { margin-top: 12px; }
</style>
