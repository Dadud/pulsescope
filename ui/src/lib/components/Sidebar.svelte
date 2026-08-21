<script lang="ts">
  import { browser } from '$app/environment';
  import { page } from '$app/state';
  import {
    primaryNavItems,
    secondaryNavSections,
    isRouteActive,
    secondarySectionIdForRoute,
  } from '$lib/navigation';

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
  /** User toggles override auto-open for the current route's group. */
  let userOpen = $state<Record<string, boolean>>({});

  $effect(() => {
    if (!browser) return;
    const sync = () => {
      const hash = window.location.hash.slice(1) || '/';
      currentRoute = hash.startsWith('/') ? hash : `/${hash}`;
      userOpen = {};
    };
    sync();
    window.addEventListener('hashchange', sync);
    const path = page.url.pathname;
    if (path && path !== '/') currentRoute = path;
    return () => window.removeEventListener('hashchange', sync);
  });

  const routeSectionId = $derived(secondarySectionIdForRoute(currentRoute));

  function linkActive(href: string): boolean {
    return isRouteActive(currentRoute, href);
  }

  function sectionOpen(id: string): boolean {
    if (Object.hasOwn(userOpen, id)) return userOpen[id];
    return routeSectionId === id;
  }

  function toggleSection(id: string) {
    userOpen = { ...userOpen, [id]: !sectionOpen(id) };
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
    <ul class="nav-list primary">
      {#each primaryNavItems as item (item.href)}
        <li>
          <a
            href={item.href}
            class:active={linkActive(item.href)}
            aria-current={linkActive(item.href) ? 'page' : undefined}
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

    {#each secondaryNavSections as section (section.id)}
      {@const open = sectionOpen(section.id)}
      {@const containsCurrent = routeSectionId === section.id}
      <div class="nav-group" class:open class:current={containsCurrent}>
        <button
          type="button"
          class="group-toggle"
          aria-expanded={open}
          aria-controls={`nav-group-${section.id}`}
          title={collapsed ? section.label : undefined}
          onclick={() => {
            if (collapsed) {
              onToggleCollapse?.();
              userOpen = { ...userOpen, [section.id]: true };
              return;
            }
            toggleSection(section.id);
          }}
        >
          {#if collapsed}
            <span class="nav-abbr">{section.label.slice(0, 2)}</span>
          {:else}
            <span class="group-label">{section.label}</span>
            <span class="group-chevron" aria-hidden="true">{open ? '▾' : '▸'}</span>
          {/if}
        </button>
        {#if open && !collapsed}
          <ul class="nav-list nested" id={`nav-group-${section.id}`}>
            {#each section.items as item (item.href)}
              <li>
                <a
                  href={item.href}
                  class:active={linkActive(item.href)}
                  aria-current={linkActive(item.href) ? 'page' : undefined}
                  title={item.description ?? item.label}
                >
                  {item.label}
                </a>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/each}
  </nav>

  <div class="sidebar-foot">
    {#if onOpenCommandPalette}
      <button type="button" class="palette-btn" onclick={onOpenCommandPalette} title="Quick jump (Ctrl+K)">
        {#if collapsed}⌘{/if}
        {#if !collapsed}<span>Jump to page</span><kbd>Ctrl K</kbd>{/if}
      </button>
    {/if}
  </div>
</aside>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    width: 204px;
    min-width: 204px;
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
    padding: 8px 6px 12px;
  }
  .nav-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .nav-list a,
  .group-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    color: var(--fg-dim);
    text-decoration: none;
    padding: 7px 10px;
    border-radius: 6px;
    font-size: 13px;
    line-height: 1.2;
    border: 1px solid transparent;
    background: transparent;
    white-space: nowrap;
    text-align: left;
  }
  .nav-list a:hover,
  .group-toggle:hover {
    background: var(--bg-elev-2);
    color: var(--fg);
  }
  .nav-list a.active {
    background: var(--bg-elev-2);
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .nav-list.nested a {
    padding-left: 16px;
    font-size: 12.5px;
  }
  .nav-group {
    margin-top: 8px;
  }
  .group-toggle {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--fg-dim);
    cursor: pointer;
  }
  .nav-group.current .group-toggle {
    color: var(--fg);
  }
  .group-label { min-width: 0; }
  .group-chevron {
    font-size: 10px;
    color: var(--fg-dim);
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
