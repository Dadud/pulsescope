<script lang="ts">
  import '../app.css';
  let { children } = $props();
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
    {@render children?.()}
  </main>
</div>
