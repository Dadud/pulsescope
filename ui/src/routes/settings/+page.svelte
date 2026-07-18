<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  let cfg = $state<any>(null);
  let status = $state<any>(null);
  let saving = $state(false);
  let saved = $state(false);
  let banks = $state<any[]>([]);
  let devices = $state<any[]>([]);
  let deviceError = $state('');
  let caps = $state<any>(null);

  async function refreshCaps() { try { caps = await Api.deviceCapabilities(); } catch (e) { deviceError = String(e); } }
  async function control(name: string, value: string | number | boolean) { try { const r = await Api.deviceControl(name, value); caps = r.capabilities; } catch (e) { deviceError = String(e); } }

  onMount(async () => {
    try {
      cfg = await Api.settings();
      status = await Api.deviceStatus();
      const deviceResult = await Api.devices();
      devices = deviceResult.devices ?? [];
      const active = devices.find((d) => d.key === `${status.driver === 'mock' ? 'driver=mock' : status.driver}`) ?? devices.find((d) => d.driver === status.driver);
      if (active) { deviceKey = active.key; deviceLabel = active.label; }
      banks = await Api.banks();
    } catch (e) { console.warn(e); }
  });

  async function save() {
    saving = true; saved = false;
    try { await Api.setSettings(cfg); saved = true; } catch (e) { console.warn(e); }
    saving = false;
    setTimeout(() => (saved = false), 1500);
  }

  let deviceKey = $state('driver=mock');
  let deviceLabel = $state('Mock Source (Test Tones)');
  async function connect() {
    deviceError = '';
    try { await Api.deviceConnect(deviceKey, deviceLabel); status = await Api.deviceStatus(); await refreshCaps(); }
    catch (e) { deviceError = String(e); }
  }
  async function saveBank(bank: any) {
    try { const result = await Api.updateChannelBank(bank.name, { enabled: bank.enabled, dwell_ms: Number(bank.dwell_ms), hold_ms: Number(bank.hold_ms), max_vfos: Number(bank.max_vfos), squelch_db: Number(bank.squelch_db) }); const index = banks.findIndex((item) => item.name === bank.name); if (index >= 0) banks[index] = result.bank; } catch (e) { console.warn(e); }
  }
</script>

<div class="settings-page">
  <h1>Settings</h1>

  <section class="card">
    <h2>Device</h2>
    {#if status}
      <div class="row"><span>Status:</span><b>{status.connected ? 'Connected' : 'Disconnected'}</b></div>
      <div class="row"><span>Driver:</span><code>{status.driver}</code></div>
      <div class="row"><span>Sample rate:</span><code>{(status.sample_rate / 1e6).toFixed(2)} Msps</code></div>
    {/if}
    <div class="row">
      <select bind:value={deviceKey} aria-label="SDR device" onchange={() => { const d = devices.find((item) => item.key === deviceKey); if (d) deviceLabel = d.label; }} style="flex:1">
        {#each devices as device}
          <option value={device.key}>{device.label} · {device.key}</option>
        {/each}
      </select>
      <input bind:value={deviceLabel} placeholder="Label" style="flex:1" />
      <button class="primary" onclick={connect}>{status?.connected ? 'Reconnect' : 'Connect'}</button>
    </div>
    {#if deviceError}<div class="device-error" role="alert">{deviceError}</div>{/if}
  </section>

  {#if caps?.connected}
    <section class="card">
      <h2>Receiver frontend</h2>
      {#if caps.agc_supported}<label class="row"><span>RF AGC</span><input type="checkbox" checked={caps.agc_enabled} onchange={(e) => control('agc', e.currentTarget.checked)} /></label>{/if}
      {#if caps.dc_offset_auto_supported}<label class="row"><span>DC offset auto</span><input type="checkbox" checked={caps.dc_offset_auto} onchange={(e) => control('dc_offset_auto', e.currentTarget.checked)} /></label>{/if}
      {#if caps.iq_balance_auto_supported}<label class="row"><span>IQ balance auto</span><input type="checkbox" checked={caps.iq_balance_auto} onchange={(e) => control('iq_balance_auto', e.currentTarget.checked)} /></label>{/if}
      {#if caps.frequency_correction_supported}<label class="row"><span>Frequency correction (PPM)</span><input type="number" step="0.1" value={caps.frequency_correction_ppm} onchange={(e) => control('frequency_correction_ppm', e.currentTarget.value)} /></label>{/if}
      {#if caps.antennas.length > 1}<label class="row"><span>Antenna</span><select value={caps.antenna} onchange={(e) => control('antenna', e.currentTarget.value)}>{#each caps.antennas as antenna}<option value={antenna}>{antenna}</option>{/each}</select></label>{/if}
      {#each caps.gain_stages as stage}
        <label class="row"><span>{stage.name} gain ({stage.value_db.toFixed(1)} dB)</span><input type="range" min={stage.min_db} max={stage.max_db} step={stage.step_db || 1} value={stage.value_db} onchange={(e) => control(`gain:${stage.name}`, e.currentTarget.value)} /></label>
      {/each}
      {#each caps.settings as setting}
        <label class="row"><span>{setting.name}</span>
          {#if setting.kind === 'bool'}<input type="checkbox" checked={setting.value === 'true'} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.checked)} />
          {:else if setting.options.length}<select value={setting.value} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.value)}>{#each setting.options as option}<option value={option}>{option}</option>{/each}</select>
          {:else if setting.kind === 'string'}<input type="text" value={setting.value} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.value)} />
          {:else}<input type="number" min={setting.min} max={setting.max} step={setting.step || 1} value={setting.value} onchange={(e) => control(`setting:${setting.key}`, e.currentTarget.value)} />{/if}
        </label>
      {/each}
      <button onclick={refreshCaps}>Refresh receiver controls</button>
    </section>
  {/if}

  <section class="card">
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
  </section>

  {#if cfg}
    <section class="card">
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
      <div class="row"><label for="hold-ms">Signal hold (ms)</label><input id="hold-ms" type="number" min="0" bind:value={cfg.scanner.hold_ms} /></div>
      <div class="row"><label for="lockout-ms">Temporary lockout (ms)</label><input id="lockout-ms" type="number" min="0" bind:value={cfg.scanner.lockout_ms} /></div>
      <div class="row"><label for="hold-audio">Hold while audio is active</label><input id="hold-audio" type="checkbox" bind:checked={cfg.scanner.scan_hold_on_audio} /></div>
      <div class="row"><label for="hold-max">Maximum audio hold (ms)</label><input id="hold-max" type="number" min="0" bind:value={cfg.scanner.scan_hold_max_ms} /></div>
      <label class="row"><span>Priority channels (Hz, comma separated)</span><input value={cfg.scanner.priority_channels_hz.join(', ')} onchange={(e) => cfg.scanner.priority_channels_hz=e.currentTarget.value.split(',').map(Number).filter(Number.isFinite)} /></label>
      <label class="row"><span>Permanent blacklist (Hz, comma separated)</span><input value={cfg.scanner.blacklist_hz.join(', ')} onchange={(e) => cfg.scanner.blacklist_hz=e.currentTarget.value.split(',').map(Number).filter(Number.isFinite)} /></label>
    </section>

    <section class="card">
      <h2>Audio</h2>
      <div class="row">
        <label for="master-volume">Master volume</label>
        <input id="master-volume" type="range" min="0" max="1" step="0.01" bind:value={cfg.audio.master_volume} />
      </div>
      <div class="row">
        <label for="output-rate">Output sample rate</label>
        <input id="output-rate" type="number" bind:value={cfg.audio.sample_rate} />
      </div>
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
  h1 { margin: 0 0 16px; }
  .card { margin-bottom: 12px; }
  .row { display: flex; align-items: center; gap: 12px; margin: 6px 0; }
  .row label, .row span { width: 140px; color: var(--fg-dim); font-size: 12px; }
  .row b, .row code { font-family: var(--mono); }
  .ok { color: var(--ok); margin-left: 8px; }
  .device-error { color: var(--danger); background: rgba(239,68,68,.12); border: 1px solid rgba(239,68,68,.35); border-radius: 4px; padding: 8px; font: 12px var(--mono); white-space: pre-wrap; }
  .bank-row { display: grid; grid-template-columns: 1.5fr 110px 110px 100px auto; gap: 6px; align-items: center; margin: 7px 0; }
  .bank-row label { color: var(--fg); font-size: 12px; }
  .bank-row input[type=number] { min-width: 0; }
</style>
