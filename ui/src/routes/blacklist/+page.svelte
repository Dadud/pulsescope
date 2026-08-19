<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  let data = $state<any>([]);
  let frequency = $state('');
  let reason = $state('');
  let error = $state('');
  let entries = $derived(Array.isArray(data) ? data : (data?.entries ?? data?.blacklist ?? []));

  function fmtHz(hz: number) {
    if (hz >= 1e6) return `${(hz / 1e6).toFixed(5)} MHz`;
    if (hz >= 1e3) return `${(hz / 1e3).toFixed(1)} kHz`;
    return `${hz} Hz`;
  }

  async function load() {
    try {
      data = await Api.blacklist();
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  async function add() {
    const hz = Number(frequency);
    if (!Number.isFinite(hz) || hz <= 0) {
      error = 'Enter a valid frequency in Hz';
      return;
    }
    await Api.blacklistAdd(hz, reason);
    frequency = '';
    reason = '';
    await load();
  }

  async function remove(entry: any) {
    await Api.blacklistRemove(Number(entry.frequency_hz ?? entry.frequency ?? entry));
    await load();
  }

  async function clear() {
    await Api.blacklistClear();
    await load();
  }

  onMount(load);
</script>

<div class="page">
  <h1>Frequency Blacklist</h1>
  <p class="muted">Skip and Lockout on Receiver write here. The scanner ignores these frequencies on the next sweep.</p>
  {#if error}<p class="error">{error}</p>{/if}
  <section class="card form">
    <input bind:value={frequency} inputmode="numeric" placeholder="Frequency (Hz)" />
    <input bind:value={reason} placeholder="Reason (optional)" />
    <button class="primary" onclick={add}>Add</button>
    <button class="danger" onclick={clear}>Clear all</button>
    <button onclick={load}>Refresh</button>
  </section>
  <section class="card">
    {#each entries as entry}
      <div class="row">
        <div>
          <b>{fmtHz(Number(entry.frequency_hz ?? 0))}</b>
          <small>{entry.reason || 'no reason'}{entry.temporary ? ' · temporary skip' : ' · lockout'}</small>
        </div>
        <button onclick={() => remove(entry)}>Remove</button>
      </div>
    {:else}
      <div class="empty">Blacklist is empty</div>
    {/each}
  </section>
</div>
<style>
  .page{padding:16px;overflow-y:auto;height:100%}
  .muted,.empty{color:var(--fg-dim)}
  .error,.danger{color:var(--danger)}
  .card{margin:12px 0;padding:14px;background:var(--bg-elev);border:1px solid var(--line);border-radius:8px}
  .form{display:flex;gap:8px;flex-wrap:wrap}
  input,button{background:var(--bg);color:var(--fg);border:1px solid var(--line);padding:7px 10px;border-radius:5px}
  .primary{color:var(--accent);border-color:var(--accent)}
  .row{display:flex;justify-content:space-between;align-items:center;gap:10px;padding:8px 0;border-top:1px solid var(--line)}
  .row div{display:grid}
  .row small{color:var(--fg-dim);font-size:11px}
  .empty{text-align:center;padding:20px}
</style>
