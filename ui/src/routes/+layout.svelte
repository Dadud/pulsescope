<script lang="ts">
  import '../app.css';
  let { children } = $props();
  let currentPath = $state('#/');
  let menuOpen = $state(false);
  let moreOpen = $state(false);

  $effect(() => {
    currentPath = window.location.hash || '#/';
    const update = () => {
      currentPath = window.location.hash || '#/';
      menuOpen = false;
      moreOpen = false;
    };
    window.addEventListener('hashchange', update);
    return () => window.removeEventListener('hashchange', update);
  });

  const primaryLinks = [
    ['#/', 'Receiver'], ['#/monitor', 'Monitor'], ['#/messages', 'Activity'],
    ['#/recording', 'Recordings'], ['#/settings', 'Hardware']
  ];
  const moreLinks = [
    ['#/signal-id', 'Signal identification'], ['#/occupancy', 'Band occupancy'],
    ['#/profiles', 'Profiles & bookmarks'],
    ['#/jobs', 'Scheduled jobs'], ['#/cases', 'Cases'],
    ['#/feature-packs', 'Decoder setup'], ['#/blacklist', 'Frequency exclusions'],
    ['#/debug', 'Diagnostics']
  ];
  const moreActive = $derived(moreLinks.some(([href]) => href === currentPath));
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
    <ul class:open={menuOpen} class="nav-links" aria-label="Primary navigation">
      {#each primaryLinks as [href, label]}
        <li><a href={href} class:active={currentPath === href} aria-current={currentPath === href ? 'page' : undefined} onclick={() => (menuOpen = false)}>{label}</a></li>
      {/each}
      <li class="more-item">
        <button class:active={moreActive} class="more-toggle" aria-expanded={moreOpen} onclick={() => (moreOpen = !moreOpen)}>More <span aria-hidden="true">▾</span></button>
        {#if moreOpen}
          <ul class="more-menu">
            {#each moreLinks as [href, label]}
              <li><a href={href} class:active={currentPath === href} aria-current={currentPath === href ? 'page' : undefined}>{label}</a></li>
            {/each}
          </ul>
        {/if}
      </li>
    </ul>
  </nav>
  <main class="content">
    {@render children?.()}
  </main>
  <nav class="mobile-tabs" aria-label="Main sections">
    {#each primaryLinks as [href, label]}
      <a href={href} class:active={currentPath === href} aria-current={currentPath === href ? 'page' : undefined}>
        <span aria-hidden="true">{href === '#/' ? '⌁' : href === '#/monitor' ? '▦' : href === '#/messages' ? '◉' : href === '#/recording' ? '●' : '⚙'}</span>
        {label}
      </a>
    {/each}
  </nav>
</div>
