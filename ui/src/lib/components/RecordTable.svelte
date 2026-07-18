<script lang="ts">
  let { rows = [], caption = 'Results', empty = 'No results are available.', loading = false }:
    { rows?: Record<string, unknown>[]; caption?: string; empty?: string; loading?: boolean } = $props();
  const columns = $derived(Array.from(new Set(rows.flatMap((row) => Object.keys(row)))));
  const label = (key: string) => key.replaceAll('_', ' ').replace(/\b\w/g, (c) => c.toUpperCase());
  const value = (item: unknown) => Array.isArray(item) ? item.join(', ') :
    item && typeof item === 'object' ? Object.entries(item).map(([k, v]) => `${label(k)}: ${String(v ?? '—')}`).join('; ') : String(item ?? '—');
</script>

<div class="table-wrap" aria-busy={loading}>
  {#if loading}<p class="state" role="status">Loading {caption.toLowerCase()}…</p>
  {:else if !rows.length}<p class="empty" role="status">{empty}</p>
  {:else}<table><caption class="sr-only">{caption}</caption><thead><tr>{#each columns as column}<th scope="col">{label(column)}</th>{/each}</tr></thead>
    <tbody>{#each rows as row}<tr>{#each columns as column}<td data-label={label(column)}>{value(row[column])}</td>{/each}</tr>{/each}</tbody></table>{/if}
</div>
