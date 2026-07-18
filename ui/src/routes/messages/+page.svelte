<script lang="ts">
  import { onMount } from 'svelte';
  import { Api, type DecodedMessage, type DecoderStatus } from '$lib/api';

  let messages = $state<DecodedMessage[]>([]);
  let filter = $state('');
  let decoders = $state<DecoderStatus[]>([]);

  onMount(async () => { [messages, decoders] = await Promise.all([Api.decodedMessages(500), Api.decoders()]); });

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
  <section class="decoder-grid" aria-label="Native decoder availability and metrics">
    {#each decoders as decoder (decoder.descriptor.protocol)}
      <div class="decoder card">
        <strong>{decoder.descriptor.protocol.toUpperCase()}</strong>
        <span class:ready={decoder.descriptor.available}>{decoder.descriptor.available ? 'Available' : 'Unavailable'}</span>
        <small>{decoder.metrics.valid_frames} valid / {decoder.metrics.frames_attempted} attempted · {decoder.metrics.checksum_failures} checksum failures</small>
        <small>{decoder.metrics.samples_received.toLocaleString()} samples · {decoder.metrics.corrected_frames} corrected</small>
        {#if decoder.metrics.last_error}<small class="error">{decoder.metrics.last_error}</small>{/if}
      </div>
    {/each}
  </section>
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
  .decoder-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: 8px; margin-bottom: 14px; }
  .decoder { display: flex; flex-direction: column; gap: 3px; padding: 10px; }
  .decoder span { color: var(--fg-dim); } .decoder span.ready { color: #5bd69f; }
  .decoder small { color: var(--fg-dim); font-family: var(--mono); } .decoder .error { color: #ff7b72; }
  .table { display: flex; flex-direction: column; gap: 0; padding: 0; overflow: hidden; }
  .row { display: grid; grid-template-columns: 160px 110px 80px 100px 1fr; gap: 8px; padding: 6px 10px; border-bottom: 1px solid var(--line); font-size: 12px; font-family: var(--mono); }
  .row.header { text-transform: uppercase; font-size: 11px; color: var(--fg-dim); background: var(--bg-elev-2); }
  .ts { color: var(--fg-dim); }
  .proto { color: var(--accent-2); text-transform: uppercase; }
  .content { word-break: break-word; white-space: pre-wrap; }
  .empty { padding: 24px; text-align: center; color: var(--fg-dim); }
</style>
