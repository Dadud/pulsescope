<script lang="ts">
  import { navSections, isRouteActive, type NavItem } from '$lib/navigation';

  let {
    open = false,
    currentRoute = '/',
    onClose,
  }: {
    open?: boolean;
    currentRoute?: string;
    onClose?: () => void;
  } = $props();

  let query = $state('');
  let selectedIndex = $state(0);
  let inputEl: HTMLInputElement | undefined = $state();

  type PaletteRow = { kind: 'item'; item: NavItem } | { kind: 'heading'; id: string; label: string };

  const rows = $derived.by((): PaletteRow[] => {
    const needle = query.trim().toLowerCase();
    const out: PaletteRow[] = [];
    for (const section of navSections) {
      const items = needle
        ? section.items.filter(
            (item) =>
              item.label.toLowerCase().includes(needle) ||
              (item.description?.toLowerCase().includes(needle) ?? false) ||
              item.href.toLowerCase().includes(needle),
          )
        : section.items;
      if (items.length === 0) continue;
      out.push({ kind: 'heading', id: section.id, label: section.label });
      for (const item of items) out.push({ kind: 'item', item });
    }
    return out;
  });

  const itemRows = $derived(
    rows.filter((row): row is Extract<PaletteRow, { kind: 'item' }> => row.kind === 'item'),
  );

  $effect(() => {
    if (open) {
      query = '';
      selectedIndex = 0;
      requestAnimationFrame(() => inputEl?.focus());
    }
  });

  $effect(() => {
    if (selectedIndex >= itemRows.length) selectedIndex = Math.max(0, itemRows.length - 1);
  });

  function close() {
    onClose?.();
  }

  function navigate(href: string) {
    window.location.hash = href.replace('#', '');
    close();
  }

  function onKeydown(event: KeyboardEvent) {
    if (!open) return;
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        selectedIndex = Math.min(itemRows.length - 1, selectedIndex + 1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        selectedIndex = Math.max(0, selectedIndex - 1);
        break;
      case 'Enter':
        event.preventDefault();
        if (itemRows[selectedIndex]) navigate(itemRows[selectedIndex].item.href);
        break;
      case 'Escape':
        event.preventDefault();
        close();
        break;
    }
  }

  function onBackdropClick(event: MouseEvent) {
    if (event.target === event.currentTarget) close();
  }

  function itemIndex(href: string): number {
    return itemRows.findIndex((row) => row.item.href === href);
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="palette-backdrop" onclick={onBackdropClick} role="presentation">
    <div class="palette" role="dialog" aria-modal="true" aria-label="Quick jump">
      <input
        bind:this={inputEl}
        class="palette-input"
        type="search"
        placeholder="Jump to any screen…"
        bind:value={query}
        aria-label="Search destinations"
      />
      <ul class="palette-list" role="listbox">
        {#each rows as row (row.kind === 'heading' ? row.id : row.item.href)}
          {#if row.kind === 'heading'}
            <li class="palette-heading">{row.label}</li>
          {:else}
            {@const i = itemIndex(row.item.href)}
            <li>
              <button
                type="button"
                class:selected={i === selectedIndex}
                class:current={isRouteActive(currentRoute, row.item.href)}
                onclick={() => navigate(row.item.href)}
                role="option"
                aria-selected={i === selectedIndex}
              >
                <span class="palette-label">{row.item.label}</span>
                {#if row.item.description}
                  <span class="palette-desc">{row.item.description}</span>
                {/if}
              </button>
            </li>
          {/if}
        {:else}
          <li class="palette-empty">No matches</li>
        {/each}
      </ul>
      <div class="palette-hint">
        <kbd>↑↓</kbd> navigate · <kbd>Enter</kbd> go · <kbd>Esc</kbd> close
      </div>
    </div>
  </div>
{/if}

<style>
  .palette-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: rgba(5, 10, 14, 0.72);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 12vh 16px 16px;
  }
  .palette {
    width: min(520px, 100%);
    background: var(--bg-elev);
    border: 1px solid var(--line-strong);
    border-radius: 10px;
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .palette-input {
    width: 100%;
    border: none;
    border-bottom: 1px solid var(--line);
    border-radius: 0;
    padding: 14px 16px;
    font-size: 15px;
    background: var(--bg);
  }
  .palette-list {
    list-style: none;
    margin: 0;
    padding: 6px;
    max-height: 360px;
    overflow-y: auto;
  }
  .palette-heading {
    padding: 10px 12px 4px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--fg-dim);
  }
  .palette-list li button {
    width: 100%;
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 12px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
  }
  .palette-list li button:hover,
  .palette-list li button.selected {
    background: var(--bg-elev-2);
    border-color: var(--line-strong);
  }
  .palette-list li button.current .palette-label {
    color: var(--accent);
  }
  .palette-label {
    font-size: 14px;
    font-weight: 500;
    color: var(--fg);
  }
  .palette-desc {
    font-size: 12px;
    color: var(--fg-dim);
  }
  .palette-empty {
    padding: 16px;
    text-align: center;
    color: var(--fg-dim);
    font-size: 13px;
  }
  .palette-hint {
    padding: 8px 12px;
    border-top: 1px solid var(--line);
    font-size: 11px;
    color: var(--fg-dim);
    text-align: center;
  }
  .palette-hint kbd {
    font-family: var(--mono);
    font-size: 10px;
    background: var(--bg);
    border: 1px solid var(--line-strong);
    border-radius: 3px;
    padding: 1px 4px;
  }
</style>
