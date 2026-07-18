<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  let { children } = $props();
  type ComponentHealth = { name: string; status: 'ok' | 'degraded' | 'failed'; detail: string };
  let components = $state<ComponentHealth[]>([]);
  let processOnline = $state(true);
  onMount(() => {
    let active = true;
    const refresh = async () => {
      try {
        const response = await fetch('/api/ready');
        const body = await response.json();
        if (active) { processOnline = true; components = body.components ?? []; }
      } catch { if (active) processOnline = false; }
    };
    refresh(); const timer = setInterval(refresh, 10_000);
    return () => { active = false; clearInterval(timer); };
  });
</script>

<svelte:head>
  <title>PulseScope</title>
</svelte:head>

<div class="app-shell">
  <nav class="topbar">
    <div class="brand">
      <svg viewBox="0 0 32 32" width="28" height="28" aria-hidden="true">
        <circle cx="16" cy="16" r="14" fill="none" stroke="currentColor" stroke-width="2" />
        <path d="M2 16 H8 L11 6 L16 26 L20 12 L23 16 H30" fill="none"
              stroke="currentColor" stroke-width="2" stroke-linejoin="round" stroke-linecap="round" />
      </svg>
      <span class="wordmark">PulseScope</span>
    </div>
    <ul class="nav-links">
      <li><a href="#/">Scanner</a></li>
      <li><a href="#/trunking">Trunking</a></li>
      <li><a href="#/messages">Messages</a></li>
      <li><a href="#/aero">Aero</a></li>
      <li><a href="#/iridium">Iridium</a></li>
      <li><a href="#/satellites">Satellites</a></li>
      <li><a href="#/hd-radio">HD Radio</a></li>
      <li><a href="#/ble">BLE</a></li>
      <li><a href="#/lora">LoRa</a></li>
      <li><a href="#/signal-id">Signal ID</a></li>
      <li><a href="#/occupancy">Occupancy</a></li>
      <li><a href="#/recording">Recording</a></li>
      <li><a href="#/jobs">Jobs</a></li>
      <li><a href="#/cases">Cases</a></li>
      <li><a href="#/aircraft">Aircraft</a></li>
      <li><a href="#/lookups">Lookups</a></li>
      <li><a href="#/feature-packs">Features</a></li>
      <li><a href="#/blacklist">Blacklist</a></li>
      <li><a href="#/debug">Debug</a></li>
      <li><a href="#/settings">Settings</a></li>
    </ul>
  </nav>
  <main class="content">
    {#if !processOnline}
      <aside class="health-banner offline" role="status">PulseScope is offline. Reconnecting…</aside>
    {:else if components.some((component) => component.status !== 'ok')}
      <aside class="health-banner degraded" role="status">
        <strong>Running with degraded components:</strong>
        {components.filter((component) => component.status !== 'ok').map((component) => `${component.name} — ${component.detail}`).join('; ')}
      </aside>
    {/if}
    {@render children?.()}
  </main>
</div>
