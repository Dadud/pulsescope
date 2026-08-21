<script lang="ts">
  import '../app.css';
  import { browser } from '$app/environment';
  import { page } from '$app/state';
  import Sidebar from '$lib/components/Sidebar.svelte';
  import CommandPalette from '$lib/components/CommandPalette.svelte';
  import DecoderAlertBridge from '$lib/components/DecoderAlertBridge.svelte';
  import { normalizeRoute, navItemForRoute } from '$lib/navigation';

  let { children } = $props();

  let sidebarCollapsed = $state(false);
  let mobileNavOpen = $state(false);
  let paletteOpen = $state(false);
  let currentRoute = $state('/');

  $effect(() => {
    if (!browser) return;
    const sync = () => {
      const hash = window.location.hash.slice(1) || '/';
      currentRoute = hash.startsWith('/') ? hash : `/${hash}`;
      mobileNavOpen = false;
    };
    sync();
    window.addEventListener('hashchange', sync);
    const path = page.url.pathname;
    if (path && path !== '/') currentRoute = path;
    return () => window.removeEventListener('hashchange', sync);
  });

  $effect(() => {
    if (!browser) return;
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        paletteOpen = true;
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  const isScanner = $derived(normalizeRoute(currentRoute) === '/');
  const pageItem = $derived(navItemForRoute(currentRoute));
</script>

<svelte:head>
  <title>PulseScope</title>
</svelte:head>

<div class="app-shell">
  {#if mobileNavOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="mobile-backdrop" onclick={() => (mobileNavOpen = false)} role="presentation"></div>
  {/if}

  <Sidebar
    collapsed={sidebarCollapsed}
    mobileOpen={mobileNavOpen}
    onOpenCommandPalette={() => (paletteOpen = true)}
    onToggleCollapse={() => (sidebarCollapsed = !sidebarCollapsed)}
  />

  <div class="main-column">
  <header class="topbar">
    <button
      type="button"
      class="mobile-menu"
      onclick={() => (mobileNavOpen = true)}
      aria-label="Open navigation"
    >☰</button>
    <div class="topbar-title">
      {#if isScanner}
        <span class="page-title">Receiver</span>
        <span class="page-subtitle">Live spectrum · waterfall · VFOs</span>
      {:else}
        <span class="page-title">{pageItem?.label ?? currentRoute.split('/').filter(Boolean).pop()?.replace(/-/g, ' ') ?? 'PulseScope'}</span>
        {#if pageItem?.description}
          <span class="page-subtitle">{pageItem.description}</span>
        {/if}
      {/if}
    </div>
    <div class="topbar-actions">
      <button type="button" class="ghost" onclick={() => (paletteOpen = true)} title="Quick jump (Ctrl+K)">
        Jump… <kbd>Ctrl K</kbd>
      </button>
      <a href="#/settings" class="settings-chip" title="Settings">⚙</a>
    </div>
  </header>

  <main class="content" class:scanner={isScanner}>
    {@render children?.()}
  </main>
  </div>
</div>

<CommandPalette
  open={paletteOpen}
  currentRoute={currentRoute}
  onClose={() => (paletteOpen = false)}
/>

<DecoderAlertBridge />

<style>
  .app-shell {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }
  .main-column {
    flex: 1;
    display: grid;
    grid-template-rows: 44px 1fr;
    min-width: 0;
    min-height: 0;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px;
    background: var(--bg);
    border-bottom: 1px solid var(--line);
    min-height: 44px;
  }
  .mobile-menu {
    display: none;
    padding: 6px 10px;
    font-size: 16px;
    line-height: 1;
    background: transparent;
    border-color: var(--line);
  }
  .topbar-title {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .page-title {
    font-size: 14px;
    font-weight: 600;
    line-height: 1.2;
  }
  .page-subtitle {
    font-size: 11px;
    color: var(--fg-dim);
    line-height: 1.2;
  }
  .topbar-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ghost {
    background: transparent;
    border-color: var(--line);
    font-size: 12px;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ghost kbd {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--fg-dim);
    background: var(--bg-elev);
    border: 1px solid var(--line-strong);
    border-radius: 3px;
    padding: 1px 4px;
  }
  .settings-chip {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 6px;
    border: 1px solid var(--line-strong);
    color: var(--fg-dim);
    text-decoration: none;
    font-size: 16px;
  }
  .settings-chip:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .content {
    overflow: auto;
    min-height: 0;
  }
  .content.scanner {
    overflow: hidden;
  }
  .mobile-backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    background: rgba(5, 10, 14, 0.55);
  }
  @media (max-width: 760px) {
    .mobile-menu { display: block; }
    .ghost kbd { display: none; }
    .ghost { padding: 6px 10px; }
  }
</style>
