<script lang="ts">
  import { onMount } from 'svelte';
  import { Api } from '$lib/api';

  let cases = $state<any[]>([]);
  let name = $state('');
  let description = $state('');
  let error = $state('');
  let busy = $state(false);
  let selected = $state<any>(); let attachments=$state<any[]>([]); let kind=$state('decoded_message'); let reference=$state(''); let note=$state('');

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
    try { await Api.deleteCase(id); await load(); }
    catch (e) { error = String(e); }
  }
  async function openCase(c:any){selected=c;attachments=await Api.caseAttachments(c.id)}
  async function attach(){if(!selected||!reference.trim())return;let r=await Api.attachToCase(selected.id,{kind,ref:reference.trim(),note});if(!r.ok){error=r.error;return}reference='';note='';await openCase(selected)}
  async function detach(id:number){await Api.deleteAttachment(id);await openCase(selected)}
  async function evidence(){let data=await Api.evidenceExport(selected.id);let a=document.createElement('a');a.href=URL.createObjectURL(new Blob([JSON.stringify(data,null,2)],{type:'application/json'}));a.download=`case-${selected.id}-evidence.json`;a.click();URL.revokeObjectURL(a.href)}
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
        <div><button class="link" onclick={()=>openCase(c)}><b>{c.name}</b></button> <span class="status">{c.status}</span><div class="muted">{c.description}</div></div>
        <button class="danger" onclick={() => remove(c.id)}>Delete</button>
      </div>
    {:else}
      <div class="empty">No cases yet</div>
    {/each}
  </section>
  {#if selected}<section class="card detail"><div class="heading"><h2>{selected.name}: attachments</h2><button onclick={evidence}>Export verified evidence</button></div>
    <div class="form"><select bind:value={kind}><option value="decoded_message">Message</option><option value="signal_event">Signal event</option><option value="recording">Recording</option><option value="track">Track</option><option value="note">Note</option><option value="lookup_result">Lookup result</option></select><input bind:value={reference} placeholder="ID, path, or note text"><input bind:value={note} placeholder="Attachment note"><button class="primary" onclick={attach}>Attach</button></div>
    {#each attachments as a}<div class="case-row"><div><b>{a.kind}</b> · <code>{a.ref}</code><div class="muted">{a.note} · {new Date(a.attached_ms).toLocaleString()}</div></div><button onclick={()=>detach(a.id)}>Remove</button></div>{:else}<div class="empty">No attachments</div>{/each}
  </section>{/if}
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
  .link{background:none;border:0;padding:0;color:var(--accent);cursor:pointer}.heading{display:flex;justify-content:space-between;align-items:center}.detail{margin-top:16px}code{color:var(--fg-dim)}
</style>
