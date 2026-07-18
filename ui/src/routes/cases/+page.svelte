<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  let cases = $state<any[]>([]);
  let name = $state('');
  let description = $state('');
  let error = $state('');
  let busy = $state(false);

  async function load() {
    try { cases = await Api.cases(); error = ''; }
    catch (e) { error = String(e); }
  }
  async function create() {
    if (!name.trim()) { error = 'Case name is required'; return; }
    busy = true;
    try {
      await Api.createCase({ name: name.trim(), description, status: 'open', tags: '' });
      name = ''; description = ''; await load();
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }
  async function remove(id: number) {
    if (!confirm('Delete this case and its saved metadata? This cannot be undone.')) return;
    try { await Api.deleteCase(id); await load(); }
    catch (e) { error = String(e); }
  }
  onMount(load);
</script>

<div class="cases-page">
  <h1>Cases</h1>
  <p class="muted">Group recordings, messages, and notes into investigation cases.</p>
  {#if error}<p class="error">{error}</p>{/if}
  <section class="card form">
    <input bind:value={name} placeholder="Case name" onkeydown={(e) => e.key === 'Enter' && create()} />
    <input bind:value={description} placeholder="Description (optional)" />
    <button class="primary" disabled={busy} onclick={create}>Create case</button>
    <button onclick={load}>Refresh</button>
  </section>
  <section class="card">
    {#each cases as c (c.id)}
      <div class="case-row">
        <div><b>{c.name}</b> <span class="status">{c.status}</span><div class="muted">{c.description}</div></div>
        <button class="danger" onclick={() => remove(c.id)}>Delete</button>
      </div>
    {:else}
      <div class="empty">No cases yet</div>
    {/each}
  </section>
</div>

<style>
  .cases-page { padding: 16px; overflow-y: auto; height: 100%; }
  .muted { color: var(--fg-dim); }
  .form { display: flex; gap: 8px; flex-wrap: wrap; margin: 16px 0; }
  .form input { flex: 1; min-width: 180px; }
  .case-row { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 10px 0; border-bottom: 1px solid var(--line); }
  .status { color: var(--accent); margin-left: 8px; font-size: 11px; text-transform: uppercase; }
  .empty { color: var(--fg-dim); text-align: center; padding: 24px; }
  .error { color: var(--danger, #f66); }
</style>
