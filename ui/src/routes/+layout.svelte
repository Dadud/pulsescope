<script lang="ts">
  import '../app.css';
  let { children } = $props();
  let currentPath = $state('#/');
  let menuOpen = $state(false);

  $effect(() => {
    currentPath = window.location.hash || '#/';
    const update = () => (currentPath = window.location.hash || '#/');
    window.addEventListener('hashchange', update);
    return () => window.removeEventListener('hashchange', update);
  });

  const links = [
    ['#/', 'Scanner'], ['#/messages', 'Messages'], ['#/signal-id', 'Signal ID'],
    ['#/occupancy', 'Occupancy'], ['#/recording', 'Recording'], ['#/jobs', 'Jobs'],
    ['#/cases', 'Cases'], ['#/feature-packs', 'Features'], ['#/blacklist', 'Blacklist'],
    ['#/debug', 'Debug'], ['#/settings', 'Settings']
  ];
</script>

<svelte:head>
  <title>PulseScope</title>
</svelte:head>

<div class="app-shell">
  <nav class="topbar">
    <button class="menu-toggle" aria-label={menuOpen ? 'Close navigation' : 'Open navigation'} aria-expanded={menuOpen} onclick={() => (menuOpen = !menuOpen)}>☰</button>
    <div class="brand">
      <svg viewBox="0 0 32 32" width="28" height="28" aria-hidden="true">
        <circle cx="16" cy="16" r="14" fill="none" stroke="currentColor" stroke-width="2" />
        <path d="M2 16 H8 L11 6 L16 26 L20 12 L23 16 H30" fill="none"
              stroke="currentColor" stroke-width="2" stroke-linejoin="round" stroke-linecap="round" />
      </svg>
      <span class="wordmark">PulseScope</span>
    </div>
    <ul class:open={menuOpen} class="nav-links">
      {#each links as [href, label]}
        <li><a href={href} class:active={currentPath === href} aria-current={currentPath === href ? 'page' : undefined} onclick={() => (menuOpen = false)}>{label}</a></li>
      {/each}
    </ul>
  </nav>
  <main class="content">
    {@render children?.()}
  </main>
</div>
