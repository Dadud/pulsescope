<script lang="ts">
  import { browser } from '$app/environment';
  import { page } from '$app/state';
  import { navSections, isRouteActive } from '$lib/navigation';

  let {
    collapsed = false,
    mobileOpen = false,
    onOpenCommandPalette,
    onToggleCollapse,
  }: {
    collapsed?: boolean;
    mobileOpen?: boolean;
    onOpenCommandPalette?: () => void;
    onToggleCollapse?: () => void;
  } = $props();

  let currentRoute = $state('/');

  $effect(() => {
    if (!browser) return;
    const sync = () => {
      const hash = window.location.hash.slice(1) || '/';
      currentRoute = hash.startsWith('/') ? hash : `/${hash}`;
    };
    sync();
    window.addEventListener('hashchange', sync);
    // SvelteKit hash router may update pathname without hashchange in some cases.
    const path = page.url.pathname;
    if (path && path !== '/') currentRoute = path;
    return () => window.removeEventListener('hashchange', sync);
  });

  function linkActive(href: string): boolean {
    return isRouteActive(currentRoute, href);
  }
</script>

<aside class="sidebar" class:collapsed class:mobile-open={mobileOpen} aria-label="Main navigation">
  <div class="sidebar-head">
  <a href="#/" class="brand" title="PulseScope Scanner">
    <svg viewBox="0 0 32 32" width="26" height="26" aria-hidden="true">
      <circle cx="16" cy="16" r="14" fill="none" stroke="currentColor" stroke-width="2" />
      <path d="M2 16 H8 L11 6 L16 26 L20 12 L23 16 H30" fill="none"
            stroke="currentColor" stroke-width="2" stroke-linejoin="round" stroke-linecap="round" />
    </svg>
    {#if !collapsed}<span class="wordmark">PulseScope</span>{/if}
  </a>
  {#if onToggleCollapse}
    <button type="button" class="collapse-btn" onclick={onToggleCollapse} aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}>
      {collapsed ? '»' : '«'}
    </button>
  {/if}
  </div>

  <nav class="sidebar-nav">
    {#each navSections as section (section.id)}
      <div class="nav-section">
        {#if !collapsed}<div class="section-label">{section.label}</div>{/if}
        <ul>
          {#each section.items as item (item.href)}
            <li>
              <a
                href={item.href}
                class:active={linkActive(item.href)}
                title={collapsed ? item.label : item.description ?? item.label}
              >
                {#if collapsed}
                  <span class="nav-abbr">{item.label.slice(0, 2)}</span>
                {:else}
                  {item.label}
                {/if}
              </a>
            </li>
          {/each}
        </ul>
      </div>
    {/each}
  </nav>

  <div class="sidebar-foot">
    {#if onOpenCommandPalette}
      <button type="button" class="palette-btn" onclick={onOpenCommandPalette} title="Quick jump (Ctrl+K)">
        {#if collapsed}⌘{/if}
        {#if !collapsed}<span>Quick jump</span><kbd>Ctrl K</kbd>{/if}
      </button>
    {/if}
  </div>
</aside>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    width: 220px;
    min-width: 220px;
    background: var(--bg-elev);
    border-right: 1px solid var(--line);
    height: 100%;
    overflow: hidden;
  }
  .sidebar.collapsed {
    width: 52px;
    min-width: 52px;
  }
  .sidebar-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 4px;
    padding: 10px 8px;
    border-bottom: 1px solid var(--line);
    min-height: 48px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--accent);
    text-decoration: none;
    min-width: 0;
  }
  .wordmark {
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--fg);
    font-size: 14px;
    white-space: nowrap;
  }
  .collapse-btn {
    flex-shrink: 0;
    padding: 4px 8px;
    font-size: 12px;
    line-height: 1;
    background: transparent;
    border-color: var(--line);
  }
  .sidebar-nav {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 8px 6px;
  }
  .nav-section + .nav-section { margin-top: 10px; }
  .section-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--fg-dim);
    padding: 2px 8px 4px;
  }
  .nav-section ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .nav-section a {
    display: block;
    color: var(--fg-dim);
    text-decoration: none;
    padding: 6px 10px;
    border-radius: 6px;
    font-size: 13px;
    border: 1px solid transparent;
    white-space: nowrap;
  }
  .nav-section a:hover {
    background: var(--bg-elev-2);
    color: var(--fg);
  }
  .nav-section a.active {
    background: var(--bg-elev-2);
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .nav-abbr {
    font-size: 11px;
    font-weight: 600;
    font-family: var(--mono);
    text-transform: uppercase;
  }
  .sidebar-foot {
    padding: 8px 6px;
    border-top: 1px solid var(--line);
  }
  .palette-btn {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    background: var(--bg-elev-2);
    font-size: 12px;
  }
  .palette-btn kbd {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--fg-dim);
    background: var(--bg);
    border: 1px solid var(--line-strong);
    border-radius: 4px;
    padding: 2px 5px;
  }
  @media (max-width: 760px) {
    .sidebar {
      position: fixed;
      z-index: 40;
      left: 0;
      top: 0;
      bottom: 0;
      width: min(280px, 88vw);
      min-width: 0;
      transform: translateX(-105%);
      transition: transform 0.2s ease;
      box-shadow: none;
    }
    .sidebar.mobile-open {
      transform: translateX(0);
      box-shadow: var(--shadow);
    }
    .sidebar.collapsed {
      width: min(280px, 88vw);
    }
    .collapse-btn { display: none; }
  }
</style>
