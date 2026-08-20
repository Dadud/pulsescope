<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  interface TimelineEntry {
    kind: string;
    timestamp_ms: number;
    frequency_hz: number;
    protocol: string;
    summary: string;
    detail: string;
  }

  let entries = $state<TimelineEntry[]>([]);
  let hours = $state(24);
  let error = $state('');
  let filter = $state('');

  function fmtHz(hz: number) {
    if (hz >= 1e6) return `${(hz / 1e6).toFixed(4)} MHz`;
    if (hz >= 1e3) return `${(hz / 1e3).toFixed(1)} kHz`;
    return `${hz} Hz`;
  }

  function fmtTime(ms: number) {
    return new Date(ms).toLocaleString();
  }

  const filtered = $derived(
    filter
      ? entries.filter((entry) =>
          [entry.kind, entry.protocol, entry.summary, entry.detail]
            .join(' ')
            .toLowerCase()
            .includes(filter.toLowerCase()),
        )
      : entries,
  );

  async function load() {
    try {
      const data = await Api.activityTimeline(hours, 300);
      entries = Array.isArray(data?.entries) ? data.entries : [];
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    void load();
    const timer = window.setInterval(() => void load(), 15000);
    return () => window.clearInterval(timer);
  });
</script>

<svelte:head><title>Activity · PulseScope</title></svelte:head>

<div class="page">
  <header class="heading">
    <div>
      <h1>RF activity timeline</h1>
      <p class="muted">Cross-protocol view of decoded messages and signal events over the selected window.</p>
    </div>
    <div class="actions">
      <label>
        <span>Window</span>
        <select bind:value={hours} onchange={load}>
          <option value={6}>6 hours</option>
          <option value={12}>12 hours</option>
          <option value={24}>24 hours</option>
          <option value={48}>48 hours</option>
        </select>
      </label>
      <button onclick={load}>Refresh</button>
    </div>
  </header>

  {#if error}<p class="error">{error}</p>{/if}

  <input bind:value={filter} placeholder="Filter protocol, summary, or class…" />

  <div class="timeline">
    {#each filtered as entry (entry.timestamp_ms + entry.kind + entry.summary)}
      <article class="item" class:novel={entry.kind === 'novel_signal'}>
        <div class="meta">
          <span class="kind">{entry.kind}</span>
          <span class="time">{fmtTime(entry.timestamp_ms)}</span>
        </div>
        <div class="body">
          <b>{entry.protocol || entry.kind}</b>
          <span>{fmtHz(Number(entry.frequency_hz ?? 0))}</span>
          <p>{entry.summary || entry.detail || '—'}</p>
          {#if entry.detail && entry.detail !== entry.summary}
            <small>{entry.detail}</small>
          {/if}
        </div>
      </article>
    {:else}
      <div class="empty">No activity in this window yet.</div>
    {/each}
  </div>
</div>

<style>
  .page { padding: 16px; overflow-y: auto; height: 100%; max-width: 960px; }
  .heading { display: flex; justify-content: space-between; gap: 12px; flex-wrap: wrap; margin-bottom: 12px; }
  .muted, small, .empty { color: var(--fg-dim); }
  .error { color: var(--danger); }
  .actions { display: flex; gap: 8px; align-items: end; }
  .actions label { display: grid; gap: 4px; font-size: 11px; color: var(--fg-dim); }
  select, button, input { background: var(--bg); color: var(--fg); border: 1px solid var(--line); padding: 7px 11px; border-radius: 5px; }
  input { width: 100%; margin-bottom: 12px; }
  .timeline { display: grid; gap: 8px; }
  .item { padding: 12px; background: var(--bg-elev); border: 1px solid var(--line); border-radius: 8px; }
  .item.novel { border-color: var(--accent); }
  .meta { display: flex; justify-content: space-between; gap: 8px; font: 11px var(--mono); color: var(--fg-dim); margin-bottom: 6px; }
  .kind { text-transform: uppercase; color: var(--accent-2); }
  .body { display: grid; gap: 4px; font: 12px var(--mono); }
  .body p { margin: 0; color: var(--fg); white-space: pre-wrap; word-break: break-word; }
  .empty { text-align: center; padding: 24px; }
</style>
