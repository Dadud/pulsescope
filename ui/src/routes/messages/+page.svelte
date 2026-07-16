<script lang="ts">
  import { onMount } from 'svelte';
  import { Api, type DecodedMessage } from '$lib/api';

  let messages = $state<DecodedMessage[]>([]);
  let filter = $state('');

  onMount(async () => { messages = await Api.decodedMessages(500); });

  let filtered = $derived(
    filter
      ? messages.filter((m) =>
          [m.protocol, m.address, m.content].some((s) => s.toLowerCase().includes(filter.toLowerCase()))
        )
      : messages
  );

  function fmtTime(ms: number) { return new Date(ms).toLocaleString(); }
  function fmtHz(hz: number) { return hz >= 1e6 ? (hz / 1e6).toFixed(3) + ' MHz' : hz + ' Hz'; }
</script>

<div class="messages-page">
  <h1>Decoded Messages</h1>
  <input bind:value={filter} placeholder="filter by protocol, address, content…" style="width: 100%; margin-bottom: 8px;" />
  <div class="table card">
    <div class="row header">
      <span>Time</span><span>Frequency</span><span>Protocol</span><span>Address</span><span>Content</span>
    </div>
    {#each filtered as m (m.id)}
      <div class="row">
        <span class="ts">{fmtTime(m.timestamp_ms)}</span>
        <span class="freq">{fmtHz(m.frequency_hz)}</span>
        <span class="proto">{m.protocol}</span>
        <span class="addr">{m.address || '—'}</span>
        <span class="content">{m.content}</span>
      </div>
    {:else}
      <div class="empty">No messages</div>
    {/each}
  </div>
</div>

<style>
  .messages-page { padding: 16px; overflow-y: auto; height: 100%; }
  .table { display: flex; flex-direction: column; gap: 0; padding: 0; overflow: hidden; }
  .row { display: grid; grid-template-columns: 160px 110px 80px 100px 1fr; gap: 8px; padding: 6px 10px; border-bottom: 1px solid var(--line); font-size: 12px; font-family: var(--mono); }
  .row.header { text-transform: uppercase; font-size: 11px; color: var(--fg-dim); background: var(--bg-elev-2); }
  .ts { color: var(--fg-dim); }
  .proto { color: var(--accent-2); text-transform: uppercase; }
  .content { word-break: break-word; white-space: pre-wrap; }
  .empty { padding: 24px; text-align: center; color: var(--fg-dim); }
</style>
