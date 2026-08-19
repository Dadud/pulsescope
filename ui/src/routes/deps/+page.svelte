<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  type Decoder = {
    name: string;
    description: string;
    protocol: string;
    found: boolean;
    path: string | null;
    source: string;
    input_type: string;
    github_url: string | null;
    install_url: string | null;
    can_auto_install?: boolean;
    feature_pack_id?: string | null;
  };
  type Adaptation = {
    catalog_id: string;
    name: string;
    integration: string;
    readiness: string;
    native_rust: boolean;
    depmanager_name?: string | null;
    system_packages: string[];
    install_hint: string;
    discovered: boolean;
    discovered_path?: string | null;
    can_auto_install: boolean;
    notes: string;
  };
  let decoders = $state<Decoder[]>([]);
  let adaptations = $state<Adaptation[]>([]);
  let runtime = $state<any[]>([]);
  let error = $state('');
  let installGuide = $state<{ name: string; guide: string } | null>(null);
  let busy = $state<string | null>(null);

  async function load() {
    try {
      const [status, adaptationData] = await Promise.all([
        Api.sidecarsStatus(),
        Api.decoderAdaptations(),
      ]);
      runtime = status.runtime ?? [];
      decoders = status.discovered ?? [];
      adaptations = adaptationData.adaptations ?? [];
      error = '';
    } catch (e) {
      try {
        decoders = (await Api.decoderScan()) as Decoder[];
        const adaptationData = await Api.decoderAdaptations();
        adaptations = adaptationData.adaptations ?? [];
      } catch (inner) { error = String(inner); }
    }
  }

  async function fetchGuide(name: string) {
    busy = `guide-${name}`;
    try {
      const data = await Api.decoderInstallGuide(name);
      if (data.guide) installGuide = { name, guide: data.guide };
      else installGuide = null;
    } catch (e) { error = String(e); }
    finally { busy = null; }
  }

  async function installDecoder(name: string) {
    busy = `install-${name}`;
    try {
      const data = await Api.decoderInstall(name);
      if (!data.ok) throw new Error(data.error ?? 'install failed');
      await load();
    } catch (e) { error = String(e); }
    finally { busy = null; }
  }

  async function configureAll() {
    busy = 'configure';
    try {
      await Api.decoderConfigure();
      await load();
    } catch (e) { error = String(e); }
    finally { busy = null; }
  }

  onMount(load);
</script>

<div class="deps-page">
  <div class="head">
    <div>
      <h1>Decoder Dependencies</h1>
      <p class="muted">PulseScope spawns external decoder tools for POCSAG, APRS, ACARS, ADS-B, AIS etc. Install missing tools into the data directory or point config at an existing binary.</p>
    </div>
    <div class="head-actions">
      <button disabled={busy === 'configure'} onclick={configureAll}>Configure from discovery</button>
      <button onclick={load}>Refresh</button>
    </div>
  </div>
  {#if error}<p class="error">{error}</p>{/if}

  <section class="card">
    <h2>Discovered decoders</h2>
    <table>
      <thead><tr><th>Name</th><th>Protocol</th><th>Input</th><th>Status</th><th>Path / Source</th><th>Actions</th></tr></thead>
      <tbody>
        {#each decoders as d (d.name)}
          <tr class:ok={d.found} class:missing={!d.found}>
            <td><b>{d.name}</b><br /><small>{d.description}</small></td>
            <td>{d.protocol}</td>
            <td><small>{d.input_type}</small></td>
            <td><b>{d.found ? 'FOUND' : 'MISSING'}</b></td>
            <td><small>{d.path ?? '—'}</small><br /><small>source: {d.source}</small></td>
            <td class="actions">
              {#if d.can_auto_install}
                <button class="mini" disabled={busy === `install-${d.name}`} onclick={() => installDecoder(d.name)}>Install</button>
              {:else if d.install_url}
                <a href={d.install_url} target="_blank" rel="noreferrer">Download</a>
              {:else}
                <small>bundled / manual</small>
              {/if}
              {#if d.github_url}
                <button class="mini" disabled={busy === `guide-${d.name}`} onclick={() => fetchGuide(d.name)}>Guide</button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>

  <section class="card">
    <h2>Adaptable catalog decoders</h2>
    <p class="muted">Native Rust paths ship in-tree; sidecars adapt when a distro package or downloaded binary is present.</p>
    <table>
      <thead><tr><th>Catalog</th><th>Integration</th><th>Readiness</th><th>System install</th><th>Status</th></tr></thead>
      <tbody>
        {#each adaptations as entry (entry.catalog_id)}
          <tr>
            <td><b>{entry.name}</b><br /><small>{entry.catalog_id}</small></td>
            <td><small>{entry.integration}</small>{#if entry.native_rust}<br /><small>native Rust</small>{/if}</td>
            <td>{entry.readiness}</td>
            <td><small>{entry.install_hint || '—'}</small></td>
            <td>
              {#if entry.native_rust}
                <b>built-in</b>
              {:else if entry.discovered}
                <b>found</b><br /><small>{entry.discovered_path ?? ''}</small>
              {:else}
                missing
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </section>

  {#if installGuide}
    <section class="card">
      <h2>Install: {installGuide.name}</h2>
      <pre>{installGuide.guide}</pre>
    </section>
  {/if}

  <section class="card">
    <h2>Decoders currently running</h2>
    {#if runtime.length === 0}<div class="empty">No decoder processes active</div>{:else}
      <table>
        <thead><tr><th>Name</th><th>PID</th><th>Input bytes</th><th>Input samples</th><th>Exit</th></tr></thead>
        <tbody>
          {#each runtime as r (r.name)}
            <tr><td><b>{r.name}</b></td><td>{r.pid ?? '—'}</td><td>{r.input_bytes}</td><td>{r.input_samples}</td><td>{r.exit_code ?? '—'}</td></tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</div>

<style>
  .deps-page { padding: 16px; overflow-y: auto; height: 100%; max-width: 1100px; }
  h1 { margin: 0 0 8px; }
  .muted { color: var(--fg-dim); }
  .error { color: var(--danger); }
  .head { display: flex; justify-content: space-between; align-items: flex-start; gap: 12px; margin-bottom: 12px; }
  .head-actions { display: flex; gap: 8px; flex-wrap: wrap; }
  table { width: 100%; border-collapse: collapse; font-size: 12px; }
  th, td { padding: 6px 8px; border-bottom: 1px solid var(--line); text-align: left; vertical-align: top; }
  th { color: var(--fg-dim); text-transform: uppercase; font-size: 10px; letter-spacing: 0.05em; }
  tr.missing td { opacity: 0.65; }
  small { color: var(--fg-dim); }
  .mini { padding: 2px 6px; font-size: 11px; }
  .actions { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
  pre { background: var(--bg); padding: 8px; border-radius: 4px; overflow: auto; white-space: pre-wrap; font: 12px var(--mono); }
  .empty { color: var(--fg-dim); padding: 16px; text-align: center; }
  a { color: var(--accent); }
  button { background: var(--bg); color: var(--fg); border: 1px solid var(--line); padding: 7px 11px; border-radius: 5px; cursor: pointer; }
  button:disabled { opacity: 0.6; cursor: not-allowed; }
</style>
