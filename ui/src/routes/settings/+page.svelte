<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';
  import { gainStageLabel, isCommonRfSetting } from '$lib/spectrum-display';

  let cfg = $state<any>(null);
  let status = $state<any>(null);
  let saving = $state(false);
  let saved = $state(false);
  let banks = $state<any[]>([]);
  let devices = $state<any[]>([]);
  let deviceError = $state('');
  let caps = $state<any>(null);
  let expertMode = $state(false);
  let controlNotice = $state('');
  let sampleRateBusy = $state(false);

  async function refreshCaps() { try { caps = await Api.deviceCapabilities(); } catch (e) { deviceError = String(e); } }
  async function discoverDevices() {
    deviceError = '';
    try {
      const result = await Api.devices();
      devices = result.devices ?? [];
      status = await Api.deviceStatus();
      const active = devices.find((device) => device.driver === status?.driver);
      if (active) { deviceKey = active.key; deviceLabel = active.label; }
    } catch (e) { deviceError = String(e); }
  }
  function toggleExpert() {
    expertMode = !expertMode;
    localStorage.setItem('pulsescope.expert-mode', expertMode ? '1' : '0');
  }
  function sampleRateChoices() {
    const preferred = [2_000_000, 5_000_000, 8_000_000, 10_000_000];
    const ranges = caps?.sample_rate_ranges_hz ?? [];
    return preferred.filter((rate) => ranges.some((range: any) => rate >= range.minimum && rate <= range.maximum));
  }
  function gainStageLabelLocal(stage: any) {
    return gainStageLabel(stage, status?.driver);
  }
  const commonSettings = $derived((caps?.settings ?? []).filter((setting: any) => isCommonRfSetting(setting)));
  const expertSettings = $derived((caps?.settings ?? []).filter((setting: any) => !isCommonRfSetting(setting)));
  async function control(name: string, value: string | number | boolean) {
    deviceError = ''; controlNotice = 'Applying…';
    try {
      const r = await Api.deviceControl(name, value); caps = r.capabilities;
      controlNotice = `Applied and verified at ${new Date().toLocaleTimeString()}`;
    } catch (e) { deviceError = String(e); controlNotice = ''; }
  }
  async function setSampleRate(event: Event) {
    const sampleRate = Number((event.currentTarget as HTMLSelectElement).value);
    if (!sampleRate || sampleRateBusy) return;
    sampleRateBusy = true; deviceError = ''; controlNotice = 'Reconfiguring sampled spectrum…';
    try {
      const result = await Api.deviceSampleRate(sampleRate);
      status = result.status; caps = result.capabilities;
      controlNotice = `${(sampleRate / 1e6).toFixed(0)} MSPS selected · ${(Number(result.bandwidth_hz) / 1e6).toFixed(3)} MHz analog bandwidth`;
    } catch (e) { deviceError = String(e); controlNotice = ''; }
    finally { sampleRateBusy = false; }
  }

  onMount(async () => {
    expertMode = localStorage.getItem('pulsescope.expert-mode') === '1';
    try {
      cfg = await Api.settings();
      await discoverDevices();
      banks = await Api.banks();
      await refreshCaps();
    } catch (e) { console.warn(e); }
  });

  async function save() {
    saving = true; saved = false;
    try { await Api.setSettings(cfg); saved = true; } catch (e) { console.warn(e); }
    saving = false;
    setTimeout(() => (saved = false), 1500);
  }

  let deviceKey = $state('');
  let deviceLabel = $state('');
  async function connect() {
    deviceError = '';
    if (!deviceKey) { deviceError = 'No physical SDR is available. Connect hardware or choose Simulation mode explicitly.'; return; }
    try { await Api.deviceConnect(deviceKey, deviceLabel); status = await Api.deviceStatus(); await refreshCaps(); }
    catch (e) { deviceError = String(e); }
  }
  async function saveBank(bank: any) {
    try { const result = await Api.updateChannelBank(bank.name, { enabled: bank.enabled, dwell_ms: Number(bank.dwell_ms), hold_ms: Number(bank.hold_ms), max_vfos: Number(bank.max_vfos), squelch_db: Number(bank.squelch_db) }); const index = banks.findIndex((item) => item.name === bank.name); if (index >= 0) banks[index] = result.bank; } catch (e) { console.warn(e); }
  }
</script>

<div class="settings-page">
  <header class="page-heading">
    <div><h1>Hardware & settings</h1><p>Connect the radio here. PulseScope applies safe defaults automatically.</p></div>
    <button class:active={expertMode} aria-pressed={expertMode} onclick={toggleExpert}>{expertMode ? 'Exit Expert mode' : 'Expert mode'}</button>
  </header>

  <section class="card section-card">
    <header class="section-header"><div><h2>Device</h2><p class="section-lead">Connect your SDR. PulseScope applies safe RF defaults automatically.</p></div></header>
    {#if status}
      <div class="status-summary">
        <span class:ok={status.connected}>{status.connected ? '● SDR online' : '○ SDR offline'}</span>
        <span>{status.label ?? status.driver}</span>
        <span>{(status.sample_rate / 1e6).toFixed(2)} MSPS</span>
      </div>
    {/if}
    <div class="row">
      <select bind:value={deviceKey} aria-label="SDR device" onchange={() => { const d = devices.find((item) => item.key === deviceKey); if (d) deviceLabel = d.label; }} style="flex:1">
        <option value="">Select a device…</option>
        {#each devices as device}
          <option value={device.key}>{device.label} · {device.key}</option>
        {/each}
      </select>
      <button class="primary" onclick={connect}>{status?.connected ? 'Reconnect' : 'Connect'}</button>
      <button onclick={discoverDevices}>Refresh detection</button>
    </div>
    {#if devices.length === 0}<p class="empty">No SDR found. Attach USB hardware, confirm the container has USB permission, then refresh detection.</p>{/if}
    {#if deviceError}<div class="device-error" role="alert">{deviceError}</div>{/if}
    {#if expertMode && status}
      <div class="expert-summary">
        <div class="row"><span>Lifecycle</span><code>{status.lifecycle ?? 'unknown'}</code></div>
        <div class="row"><span>Driver</span><code>{status.driver}</code></div>
        <div class="row"><span>Samples received</span><code>{Number(status.stream?.samples_received ?? 0).toLocaleString()}</code></div>
        <div class="row"><span>Source errors</span><code>{Number(status.stream?.source_errors ?? 0).toLocaleString()}</code></div>
      </div>
    {/if}
  </section>

  {#if caps?.connected}
  <section class="card section-card">
    <header class="section-header">
      <div>
        <h2>Receiver frontend</h2>
        <p class="section-lead">RF gain, antenna, and driver-specific options. The same controls also appear on the Receiver while scanning.</p>
      </div>
    </header>
      <div class="row"><span>Capability contract</span><code>v{caps.contract_version}</code></div>
      <div class="row"><span>Stream MTU</span><code>{Number(caps.stream_mtu ?? 0).toLocaleString()} samples</code></div>
      <div class="row"><span>Total spectrum</span><code>{(Number(caps.total_bandwidth_hz ?? 0) / 1e6).toFixed(3)} MHz</code></div>
      <div class="row"><span>Usable spectrum</span><code>{(Number(caps.usable_bandwidth_hz ?? 0) / 1e6).toFixed(3)} MHz</code></div>
      <div class="row"><span>RX tuners</span><code>{caps.tuner_count ?? 1}</code></div>
      {#if sampleRateChoices().length}
        <label class="row"><span><b>Visible spectrum</b><small> Higher rates use more USB bandwidth and CPU.</small></span><select disabled={sampleRateBusy} value={status?.sample_rate} onchange={setSampleRate}>{#each sampleRateChoices() as rate}<option value={rate}>{(rate / 1e6).toFixed(0)} MSPS · about {(rate * 0.9 / 1e6).toFixed(1)} MHz usable</option>{/each}</select></label>
      {/if}
      {#if caps.agc_supported}<label class="row"><span><b>RF AGC</b><small> Automatic RF level; manual reductions are ignored while enabled.</small></span><input type="checkbox" checked={caps.agc_enabled} onchange={(e) => control('agc', e.currentTarget.checked)} /></label>{/if}
      {#if controlNotice}<p class="control-notice" role="status">{controlNotice}</p>{/if}
      {#if caps.antennas.length > 1}<label class="row"><span>Antenna</span><select value={caps.antenna} onchange={(e) => control('antenna', e.currentTarget.value)}>{#each caps.antennas as antenna}<option value={antenna}>{antenna}</option>{/each}</select></label>{/if}
      {#each caps.gain_stages as stage}
        <label class="row"><span>{gainStageLabelLocal(stage)} ({stage.value_db.toFixed(1)} dB){#if status?.driver === 'sdrplay'}<small> Lower reduction = more RF gain.</small>{/if}</span><input disabled={caps.agc_enabled} type="range" min={stage.min_db} max={stage.max_db} step={stage.step_db || 1} value={stage.value_db} oninput={(e) => (stage.value_db = Number(e.currentTarget.value))} onchange={(e) => control(`gain:${stage.name}`, e.currentTarget.value)} /></label>
      {/each}
      {#each commonSettings as setting}
        <label class="row"><span>{setting.name}<small>{setting.key}</small></span>
          {#if setting.kind === 'bool'}<input type="checkbox" checked={setting.value === 'true'} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.checked)} />
          {:else if setting.options.length}<select value={setting.value} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.value)}>{#each setting.options as option}<option value={option}>{option}</option>{/each}</select>
          {:else if setting.kind === 'string'}<input type="text" value={setting.value} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.value)} />
          {:else}<input type="number" min={setting.min} max={setting.max} step={setting.step || 1} value={setting.value} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.value)} />{/if}
        </label>
      {/each}
      {#if expertMode}
        <h3>Expert RF controls</h3>
        {#if caps.dc_offset_auto_supported}<label class="row"><span>DC offset auto</span><input type="checkbox" checked={caps.dc_offset_auto} onchange={(e) => control('dc_offset_auto', e.currentTarget.checked)} /></label>{/if}
        {#if caps.iq_balance_auto_supported}<label class="row"><span>IQ balance auto</span><input type="checkbox" checked={caps.iq_balance_auto} onchange={(e) => control('iq_balance_auto', e.currentTarget.checked)} /></label>{/if}
        {#if caps.frequency_correction_supported}<label class="row"><span>Frequency correction (PPM)</span><input type="number" step="0.1" value={caps.frequency_correction_ppm} onchange={(e) => control('frequency_correction_ppm', e.currentTarget.value)} /></label>{/if}
        {#each expertSettings as setting}
          <label class="row"><span>{setting.name}</span>
            {#if setting.kind === 'bool'}<input type="checkbox" checked={setting.value === 'true'} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.checked)} />
            {:else if setting.options.length}<select value={setting.value} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.value)}>{#each setting.options as option}<option value={option}>{option}</option>{/each}</select>
            {:else if setting.kind === 'string'}<input type="text" value={setting.value} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.value)} />
            {:else}<input type="number" min={setting.min} max={setting.max} step={setting.step || 1} value={setting.value} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.value)} />{/if}
          </label>
        {/each}
      {/if}
      <button onclick={refreshCaps}>Refresh receiver controls</button>
    </section>
  {/if}

  {#if expertMode}<section class="card">
    <h2>Per-band scan overrides</h2>
    {#each banks as bank (bank.name)}
      <div class="bank-row">
        <label><input type="checkbox" bind:checked={bank.enabled} /> {bank.name}</label>
        <input aria-label="{bank.name} dwell milliseconds" type="number" min="1" bind:value={bank.dwell_ms} />
        <input aria-label="{bank.name} hold milliseconds" type="number" min="0" bind:value={bank.hold_ms} />
        <input aria-label="{bank.name} squelch dB" type="number" step="0.5" bind:value={bank.squelch_db} />
        <button onclick={() => saveBank(bank)}>Save</button>
      </div>
    {/each}
  </section>{/if}

  {#if cfg}
    {#if expertMode}<section class="card">
      <h2>Scanner</h2>
      <div class="row">
        <label for="fft-size">FFT size</label>
        <input id="fft-size" type="number" bind:value={cfg.scanner.fft_size} />
      </div>
      <div class="row">
        <label for="update-rate">Update rate (Hz)</label>
        <input id="update-rate" type="number" bind:value={cfg.scanner.update_rate_hz} />
      </div>
      <div class="row">
        <label for="max-vfos">Max VFOs</label>
        <input id="max-vfos" type="number" bind:value={cfg.scanner.max_vfos} />
      </div>
      <div class="row">
        <label for="squelch">Squelch (dB)</label>
        <input id="squelch" type="number" step="0.5" bind:value={cfg.scanner.squelch_db} />
      </div>
      <div class="row">
        <label for="auto-decode-all">Auto-decode all protocols</label>
        <input id="auto-decode-all" type="checkbox" bind:checked={cfg.scanner.auto_decode_all} />
      </div>
      <div class="row">
        <label for="auto-decode-threshold">Auto-decode threshold</label>
        <input id="auto-decode-threshold" type="number" min="0" max="1" step="0.05" bind:value={cfg.scanner.auto_decode_threshold} />
      </div>
      <div class="row">
        <label for="use-arrl-bandplan">Use ARRL band-plan hints (auto mode + decode routing)</label>
        <input id="use-arrl-bandplan" type="checkbox" bind:checked={cfg.scanner.use_arrl_bandplan} />
      </div>
    </section>{/if}

    <section class="card">
      <h2>Audio</h2>
      <label class="row"><span>Broadcast FM de-emphasis<small>75 µs for the Americas; 50 µs for most other regions.</small></span><select bind:value={cfg.demodulator.de_emphasis_us}><option value={75}>75 µs</option><option value={50}>50 µs</option></select></label>
      <div class="row">
        <label for="master-volume">Master volume</label>
        <input id="master-volume" type="range" min="0" max="1" step="0.01" bind:value={cfg.audio.master_volume} />
      </div>
      {#if expertMode}<div class="row">
        <label for="output-rate">Output sample rate</label>
        <input id="output-rate" type="number" bind:value={cfg.audio.sample_rate} />
      </div>{/if}
    </section>

    <section class="card">
      <h2>Receiver location</h2>
      <div class="row"><label for="latitude">Latitude</label><input id="latitude" type="number" step="0.0001" bind:value={cfg.receiver_location.latitude_deg} /></div>
      <div class="row"><label for="longitude">Longitude</label><input id="longitude" type="number" step="0.0001" bind:value={cfg.receiver_location.longitude_deg} /></div>
      <div class="row"><label for="altitude">Altitude (m)</label><input id="altitude" type="number" bind:value={cfg.receiver_location.altitude_m} /></div>
    </section>

    <button class="primary" onclick={save} disabled={saving}>
      {saving ? 'Saving…' : 'Save'}
    </button>
    {#if saved}<span class="ok">✓ Saved</span>{/if}
  {/if}
</div>

<style>
  .settings-page { padding: 16px; overflow-y: auto; height: 100%; max-width: 720px; }
  .page-heading { display:flex; align-items:flex-start; justify-content:space-between; gap:16px; margin-bottom:16px; }
  h1 { margin: 0 0 16px; }
  .page-heading h1 { margin:0 0 3px; }
  .page-heading p, .empty { margin:0; color:var(--fg-dim); font-size:12px; }
  .page-heading button.active { color:var(--accent); border-color:var(--accent); }
  h3 { margin:16px 0 8px; padding-top:12px; border-top:1px solid var(--line); color:var(--accent-2); font-size:12px; text-transform:uppercase; letter-spacing:.05em; }
  .card { margin-bottom: 12px; }
  .row { display: flex; align-items: center; gap: 12px; margin: 6px 0; }
  .row label, .row span { width: 140px; color: var(--fg-dim); font-size: 12px; }
  .row span small { display:block; width:auto; margin-top:2px; color:var(--fg-dim); font-size:10px; line-height:1.3; }
  .row code { font-family: var(--mono); }
  .control-notice { margin:8px 0; padding:7px 9px; color:var(--ok); background:rgb(34 197 94 / 9%); border:1px solid rgb(34 197 94 / 25%); border-radius:5px; font:11px var(--mono); }
  .ok { color: var(--ok); margin-left: 8px; }
  .device-error { color: var(--danger); background: rgba(239,68,68,.12); border: 1px solid rgba(239,68,68,.35); border-radius: 4px; padding: 8px; font: 12px var(--mono); white-space: pre-wrap; }
  .bank-row { display: grid; grid-template-columns: 1.5fr 110px 110px 100px auto; gap: 6px; align-items: center; margin: 7px 0; }
  .bank-row label { color: var(--fg); font-size: 12px; }
  .bank-row input[type=number] { min-width: 0; }
  .status-summary { display:flex; flex-wrap:wrap; gap:8px 14px; margin-bottom:10px; padding:10px; background:var(--bg); border:1px solid var(--line); border-radius:7px; font:12px var(--mono); }
  .status-summary .ok { color:var(--ok); }
  .expert-summary { margin-top:10px; padding-top:8px; border-top:1px solid var(--line); }
  @media (max-width: 760px) {
    .settings-page { padding:10px; max-width:none; }
    .page-heading { align-items:stretch; flex-direction:column; }
    .row { align-items:stretch; flex-direction:column; gap:5px; }
    .row label, .row span { width:auto; }
    .row > input:not([type=checkbox]), .row > select, .row > button { width:100%; }
    .row > input[type=range] { min-height: 44px; }
    .bank-row { grid-template-columns:1fr 1fr; padding:9px 0; border-bottom:1px solid var(--line); }
    .bank-row label { grid-column:1 / -1; }
  }
</style>
