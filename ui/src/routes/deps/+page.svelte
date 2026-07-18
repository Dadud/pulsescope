<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  type Decoder = { name: string; description: string; protocol: string; found: boolean; healthy: boolean; health: string; platform_supported: boolean; installed_version: string | null; license: string; license_url: string; license_accepted: boolean; transport: string; path: string | null; source: string; input_type: string; github_url: string | null; install_url: string | null };
  let decoders = $state<Decoder[]>([]);
  let runtime = $state<any[]>([]);
  let error = $state('');
  let installGuide = $state<{ name: string; guide: string } | null>(null);
  let busy = $state('');
  const API_BASE = 'http://127.0.0.1:8765';

  async function load() {
    try {
      const status = await (await fetch(`${API_BASE}/sidecars/status`)).json();
      runtime = status.runtime ?? [];
      decoders = status.discovered ?? [];
      error = '';
    } catch (e) {
      try {
        const fallback = await (await fetch(`${API_BASE}/decoders/scan`)).json();
        decoders = fallback ?? [];
      } catch (inner) { error = String(inner); }
    }
  }

  async function fetchGuide(name: string) {
    try {
      const r = await fetch(`${API_BASE}/decoders/install/${name}`, { method: 'POST' });
      const data = await r.json();
      if (data.ok) installGuide = { name, guide: data.guide };
      else installGuide = null;
    } catch (e) { error = String(e); }
  }

  async function manage(d: Decoder, operation: 'install'|'update'|'repair'|'uninstall') {
    if (operation !== 'uninstall' && !confirm(`Accept ${d.license} (${d.license_url}) and ${operation} ${d.name}?`)) return;
    busy = d.name; error = '';
    try {
      const r = await fetch(`${API_BASE}/decoders/install/${d.name}`, { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify({ operation, accept_license: operation !== 'uninstall' }) });
      const result = await r.json(); if (!result.ok) error = result.error; await load();
    } catch (e) { error = String(e); } finally { busy = ''; }
  }

  onMount(load);
</script>

<div class="deps-page">
  <h1>Decoder Dependencies</h1>
  <p class="muted">PulseScope spawns external decoder tools for POCSAG, APRS, ACARS, ADS-B, AIS etc. Run-time status shows which decoders are alive; the discovery table identifies each decoder's presence and where it was found.</p>
  {#if error}<p class="error">{error}</p>{/if}

  <section class="card">
    <div class="head"><h2>Discovered decoders</h2><button onclick={load}>Refresh</button></div>
    <table>
      <thead><tr><th>Name</th><th>Contract</th><th>Platform / license</th><th>Health</th><th>Version / traffic</th><th>Actions</th></tr></thead>
      <tbody>
        {#each decoders as d (d.name)}
          <tr class:ok={d.found} class:missing={!d.found}>
            <td><b>{d.name}</b><br /><small>{d.description}</small></td>
            <td>{d.protocol}<br/><small>{d.transport}</small></td>
            <td><b>{d.platform_supported ? 'SUPPORTED' : 'UNSUPPORTED'}</b><br/><a href={d.license_url} target="_blank">{d.license}</a> · {d.license_accepted ? 'accepted' : 'not accepted'}</td>
            <td><b>{d.healthy ? 'HEALTHY' : 'UNAVAILABLE'}</b><br/><small>{d.health}</small></td>
            <td>{d.installed_version ?? 'not managed'}<br/><small>{d.path ?? d.source}</small></td>
            <td><button disabled={busy===d.name || !d.platform_supported} class="mini" onclick={() => manage(d, d.installed_version ? 'update' : 'install')}>{d.installed_version ? 'Update' : 'Install'}</button> <button disabled={busy===d.name} class="mini" onclick={() => manage(d, 'repair')}>Repair</button> <button disabled={busy===d.name || !d.installed_version} class="mini" onclick={() => manage(d, 'uninstall')}>Remove</button></td>
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
        <thead><tr><th>Name</th><th>Health</th><th>PID</th><th>Traffic</th><th>Restarts</th><th>Failure</th></tr></thead>
        <tbody>
          {#each runtime as r (r.name)}
            <tr><td><b>{r.name}</b></td><td>{r.healthy ? 'healthy' : 'failed'}</td><td>{r.pid ?? '—'}</td><td>{r.input_bytes} B / {r.input_samples} samples / {r.output_messages} messages</td><td>{r.restarts}/{r.restart_limit}</td><td>{r.failure ?? (r.exit_code == null ? '—' : `exit ${r.exit_code}`)}</td></tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</div>

<style>
  .deps-page { padding: 16px; overflow-y: auto; height: 100%; max-width: 1100px; }
  h1 { margin: 0 0 12px; }
  .muted { color: var(--fg-dim); }
  .error { color: var(--danger); }
  .head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; }
  table { width: 100%; border-collapse: collapse; font-size: 12px; }
  th, td { padding: 6px 8px; border-bottom: 1px solid var(--line); text-align: left; vertical-align: top; }
  th { color: var(--fg-dim); text-transform: uppercase; font-size: 10px; letter-spacing: 0.05em; }
  tr.missing td { opacity: 0.65; }
  small { color: var(--fg-dim); }
  .mini { padding: 2px 6px; font-size: 11px; }
  pre { background: var(--bg); padding: 8px; border-radius: 4px; overflow: auto; white-space: pre-wrap; font: 12px var(--mono); }
  .empty { color: var(--fg-dim); padding: 16px; text-align: center; }
  a { color: var(--accent); }
</style>
