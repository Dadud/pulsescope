<script lang="ts">
  import '../app.css';
  import { onMount } from 'svelte';
  let { children } = $props();
  let route = $state('/');
  let open = $state(false);
  const links = [
    ['/', 'Scanner'], ['/trunking', 'Trunking'], ['/messages', 'Messages'], ['/aero', 'Aero'], ['/iridium', 'Iridium'],
    ['/satellites', 'Satellites'], ['/hd-radio', 'HD Radio'], ['/ble', 'BLE'], ['/lora', 'LoRa'], ['/signal-id', 'Signal ID'],
    ['/occupancy', 'Occupancy'], ['/recording', 'Recording'], ['/jobs', 'Jobs'], ['/cases', 'Cases'], ['/aircraft', 'Aircraft'],
    ['/lookups', 'Lookups'], ['/feature-packs', 'Features'], ['/blacklist', 'Blacklist'], ['/debug', 'Debug'], ['/settings', 'Settings']
  ];
  onMount(() => {
    const update = () => { route = location.hash.slice(1).split('?')[0] || '/'; open = false; };
    update(); addEventListener('hashchange', update);
    return () => removeEventListener('hashchange', update);
  });
</script>

<svelte:head>
  <title>PulseScope</title>
</svelte:head>

<div class="app-shell">
  <nav class="topbar" aria-label="Primary navigation">
    <div class="brand">
      <svg viewBox="0 0 32 32" width="28" height="28" aria-hidden="true">
        <circle cx="16" cy="16" r="14" fill="none" stroke="currentColor" stroke-width="2" />
        <path d="M2 16 H8 L11 6 L16 26 L20 12 L23 16 H30" fill="none"
              stroke="currentColor" stroke-width="2" stroke-linejoin="round" stroke-linecap="round" />
      </svg>
      <span class="wordmark">PulseScope</span>
    </div>
    <button class="nav-toggle" aria-expanded={open} aria-controls="primary-links" aria-label="Toggle navigation" onclick={() => open = !open}>☰</button>
    <ul class="nav-links" class:open id="primary-links">
      {#each links as link}<li><a href={`#${link[0]}`} class:active={route === link[0]} aria-current={route === link[0] ? 'page' : undefined}>{link[1]}</a></li>{/each}
    </ul>
  </nav>
  <main class="content">
    {@render children?.()}
  </main>
</div>
