<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  let packs = $state<any[]>([]);
  let decoders = $state<any[]>([]);
  let error = $state('');
  let busy = $state<string | null>(null);
  let filter = $state('');
  const visibleDecoders = $derived(decoders.filter((decoder) => `${decoder.name} ${decoder.id} ${decoder.input}`.toLowerCase().includes(filter.trim().toLowerCase())));

  async function load() {
    try {
      const [packData, catalog] = await Promise.all([Api.featurePacks(), Api.decoderCatalogV2()]);
      packs = packData.groups ?? packData.packs ?? packData ?? [];
      decoders = catalog.decoders ?? [];
      error = '';
    } catch (e) { error = String(e); }
  }
  onMount(load);

  async function toggle(pack: any) {
    busy = pack.id;
    try { await Api.featurePackEnable(pack.id, !pack.enabled); await load(); }
    catch (e) { error = String(e); }
    finally { busy = null; }
  }
</script>

<div class="page">
  <div class="heading">
    <div><h1>Decoders</h1><p class="muted">What PulseScope can decode, what is installed, and what still needs verification.</p></div>
    <button onclick={load}>Refresh</button>
  </div>
  <div class="truth-note"><b>Beta means usable for development, not verified reception.</b> A decoder becomes available only after its recorded-IQ end-to-end gate passes.</div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <input class="search" aria-label="Search decoders" placeholder="Search DMR, ADS-B, RDS…" bind:value={filter} />
  <div class="decoder-grid">
    {#each visibleDecoders as decoder (decoder.id)}
      <section class="decoder-card">
        <div class="title"><h2>{decoder.name}</h2><span class:installed={decoder.status === 'installed'} class="badge">{decoder.status === 'installed' ? 'Installed' : 'Beta'}</span></div>
        <dl><div><dt>Input</dt><dd>{decoder.input}</dd></div><div><dt>Integration</dt><dd>{decoder.integration}</dd></div></dl>
        <p class="gate"><b>Remaining gate:</b> {decoder.missing_gate ?? 'Health and recorded-IQ verification'}</p>
      </section>
    {:else}<section class="card empty">No decoders match that search.</section>{/each}
  </div>

  <h2 class="section-heading">Installed capability packs</h2>
  <p class="muted">A pack is configuration for an isolated decoder process. Enabling it does not pretend a missing executable is installed.</p>
  <div class="grid">
    {#each packs as pack}
      <section class="card">
        <div class="title"><h2>{pack.name ?? pack.id}</h2><button disabled={busy === pack.id || !pack.available} class:enabled={pack.enabled} onclick={() => toggle(pack)}>{pack.enabled ? 'Enabled' : pack.available ? 'Enable' : 'Not installed'}</button></div>
        <p class="muted">{pack.path || 'No executable configured'}</p>
        <p>Protocols: {(pack.protocols ?? []).join(', ')}</p>
        <small class:ok={pack.enabled}>{pack.availability_reason ?? (pack.enabled ? 'Configured enabled' : 'Configured disabled')}</small>
      </section>
    {:else}<section class="card empty">No capability packs reported.</section>{/each}
  </div>
</div>

<style>
  .page{padding:16px;overflow-y:auto;height:100%;max-width:1200px;margin:auto}.heading{display:flex;align-items:center;justify-content:space-between;gap:12px}.heading h1{margin:0}.muted,.empty{color:var(--fg-dim)}.error{color:var(--danger)}.truth-note{padding:10px 12px;margin:12px 0;background:rgb(245 158 11 / 10%);border:1px solid rgb(245 158 11 / 30%);border-radius:7px;color:#fcd38d;font-size:12px}.search{width:min(100%,420px);margin:0 0 12px}.decoder-grid,.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:12px}.decoder-card,.card{padding:14px;background:var(--bg-elev);border:1px solid var(--line);border-radius:8px}.decoder-card{border-left:3px solid var(--warn)}.title{display:flex;justify-content:space-between;align-items:center;gap:8px}.title h2{font-size:15px;margin:0;text-transform:none;color:var(--fg)}.badge{padding:3px 7px;border-radius:99px;background:rgb(245 158 11 / 14%);color:var(--warn);font:10px var(--mono);text-transform:uppercase}.badge.installed{color:var(--ok);background:rgb(34 197 94 / 12%)}dl{margin:10px 0;display:flex;gap:18px}dl div{min-width:0}dt{font-size:10px;color:var(--fg-dim);text-transform:uppercase}dd{margin:2px 0;font:11px var(--mono);overflow-wrap:anywhere}.gate{margin:8px 0 0;color:var(--fg-dim);font-size:11px}.section-heading{margin:24px 0 0;font-size:16px}button{background:var(--bg);color:var(--fg);border:1px solid var(--line);padding:7px 11px;border-radius:5px;cursor:pointer}.title button.enabled{color:var(--ok);border-color:var(--ok)}button:disabled{opacity:.6;cursor:not-allowed}.ok{color:var(--ok)}small{font:11px var(--mono)}p{font-size:12px}@media(max-width:760px){.page{padding:10px}.heading{align-items:stretch}.heading button{min-width:88px}.decoder-grid,.grid{grid-template-columns:1fr}.search{width:100%}}
</style>
