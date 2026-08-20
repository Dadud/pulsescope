<script lang="ts">
  import { onMount } from 'svelte';
  import { Api, type ReceiverBookmark, type ReceiverProfile } from '$lib/api';
  let profiles = $state<ReceiverProfile[]>([]), bookmarks = $state<ReceiverBookmark[]>([]);
  let status = $state<any>(null), notice = $state(''), busy = $state(false);
  let name = $state(''), mode = $state('nfm'), region = $state(''), deemphasis = $state<number | undefined>(undefined);
  const demodModes = ['am','sam','usb','lsb','cw','nfm','wfm'];
  function hz(value:number){return value>=1e6?`${(value/1e6).toFixed(5)} MHz`:`${(value/1e3).toFixed(1)} kHz`}
  async function load(){
    const [p,b,s]=await Promise.all([Api.profilesV2(),Api.bookmarksV2(),Api.deviceStatus()]); profiles=p.profiles;bookmarks=b.bookmarks;status=s;
  }
  async function save(){
    if(!name.trim()||!status)return;busy=true;notice='';
    try{await Api.saveProfileV2({name:name.trim(),center_frequency_hz:Number(status.center_freq_hz),sample_rate_hz:Number(status.sample_rate),bandwidth_hz:Number(status.bandwidth_hz||status.sample_rate),mode,region,deemphasis_us:deemphasis??null,gain_policy:{source:'capability_driven'},decoder_policy:{}});name='';await load();notice='Profile saved from the current hardware window.'}catch(e){notice=String(e)}finally{busy=false}
  }
  async function apply(profile:ReceiverProfile){if(!profile.id)return;busy=true;try{const result=await Api.applyProfileV2(profile.id);status=result.actual;notice=`Applied ${profile.name}. Open Receiver to listen using ${profile.mode.toUpperCase()}.`}catch(e){notice=String(e)}finally{busy=false}}
  async function removeProfile(profile:ReceiverProfile){if(!profile.id)return;await Api.deleteProfileV2(profile.id);profiles=profiles.filter(p=>p.id!==profile.id)}
  async function removeBookmark(bookmark:ReceiverBookmark){if(bookmark.id===undefined)return;await Api.deleteBookmarkV2(bookmark.id);bookmarks=bookmarks.filter(b=>b.id!==bookmark.id)}
  let listeningModes = $state<any[]>([]);
  async function loadModes(){try{const data=await Api.listeningModesV2();listeningModes=data?.modes??[]}catch(e){notice=String(e)}}
  async function applyMode(id:string){busy=true;notice='';try{const result=await Api.applyListeningModeV2(id);status=result.actual;notice=`Applied ${id}. Suggested decoders: ${(result.suggested_decoders??[]).join(', ')}`}catch(e){notice=String(e)}finally{busy=false}}
  onMount(()=>{void load().catch(e=>notice=String(e));void loadModes()});
</script>
<svelte:head><title>Profiles & bookmarks · PulseScope</title></svelte:head>
<section class="page">
  <header><p class="eyebrow">Receiver library</p><h1>Profiles & bookmarks</h1><p>Profiles recall the shared SDR capture window. Bookmarks recall individual channels inside a window.</p></header>
  {#if notice}<div class="notice" role="status">{notice}</div>{/if}
  <section class="card modes">
    <h2>Quick listening modes</h2>
    <p class="muted">One-tap presets tune the shared hardware window and suggest decoder packs. Unsupported values are rejected.</p>
    <div class="mode-grid">{#each listeningModes as preset}<button disabled={busy} onclick={()=>applyMode(preset.id)}><b>{preset.name}</b><small>{preset.description}</small></button>{/each}</div>
  </section>
  <div class="grid">
    <section class="card"><h2>Save current window</h2><div class="current"><b>{hz(Number(status?.center_freq_hz||0))}</b><span>{(Number(status?.sample_rate||0)/1e6).toFixed(2)} MSPS · {(Number(status?.bandwidth_hz||0)/1e6).toFixed(2)} MHz usable</span></div><div class="form"><label>Name<input bind:value={name} placeholder="Local FM" /></label><label>Default mode<select bind:value={mode}>{#each demodModes as item}<option value={item}>{item.toUpperCase()}</option>{/each}</select></label><label>Region<input bind:value={region} placeholder="US Midwest" /></label><label>FM de-emphasis<select bind:value={deemphasis}><option value={undefined}>Use server default</option><option value={75}>75 µs</option><option value={50}>50 µs</option></select></label><button class="primary" onclick={save} disabled={busy||!name.trim()||!status}>Save profile</button></div></section>
    <section class="card"><div class="section-head"><h2>Hardware profiles</h2><span>{profiles.length}</span></div><div class="list">{#each profiles as profile (profile.id)}<article><div><b>{profile.name}</b><small>{hz(profile.center_frequency_hz)} · {(profile.sample_rate_hz/1e6).toFixed(2)} MSPS · {profile.mode.toUpperCase()}</small></div><button class="primary" onclick={()=>apply(profile)} disabled={busy}>Apply</button><button aria-label={`Delete ${profile.name}`} onclick={()=>removeProfile(profile)}>Delete</button></article>{:else}<p class="empty">No profiles saved yet.</p>{/each}</div></section>
    <section class="card bookmarks"><div class="section-head"><h2>Frequency bookmarks</h2><span>{bookmarks.length}</span></div><div class="list">{#each bookmarks as bookmark (bookmark.id)}<article><div><b>{bookmark.label}</b><small>{hz(bookmark.frequency_hz)} · {bookmark.mode.toUpperCase()}</small></div><a href="#/">Open receiver</a><button aria-label={`Delete ${bookmark.label}`} onclick={()=>removeBookmark(bookmark)}>Delete</button></article>{:else}<p class="empty">Save a frequency from the Receiver sidebar.</p>{/each}</div></section>
  </div>
</section>
<style>
  .page{height:100%;overflow:auto;padding:clamp(12px,2vw,24px);max-width:1200px;margin:auto}header{margin-bottom:16px}h1{margin:2px 0;font-size:28px}header p{margin:0;color:var(--fg-dim)}.eyebrow{text-transform:uppercase;letter-spacing:.1em;font-size:10px;color:var(--accent)!important;font-weight:700}.notice{padding:10px 12px;margin-bottom:10px;border:1px solid var(--line-strong);border-radius:7px;color:var(--accent)}.modes{margin-bottom:12px}.mode-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:8px}.mode-grid button{display:grid;gap:4px;text-align:left;padding:10px;background:var(--bg);border:1px solid var(--line);border-radius:7px;color:var(--fg)}.mode-grid small{color:var(--fg-dim);font-size:11px;line-height:1.3}.grid{display:grid;grid-template-columns:minmax(280px,.8fr) minmax(400px,1.2fr);gap:12px}.bookmarks{grid-column:2}.current{display:grid;gap:2px;padding:10px;background:var(--bg);border-radius:7px;margin-bottom:10px}.current b{font:600 18px var(--mono)}.current span,.list small{color:var(--fg-dim);font-size:11px}.form{display:grid;gap:9px}.form label{display:grid;gap:3px;color:var(--fg-dim);font-size:11px}.section-head{display:flex;justify-content:space-between}.section-head span{color:var(--fg-dim)}.list article{display:grid;grid-template-columns:minmax(0,1fr) auto auto;gap:7px;align-items:center;padding:9px 0;border-top:1px solid var(--line)}.list article>div{display:grid;min-width:0}.list a{color:var(--accent-2);text-decoration:none;font-size:12px}.empty{color:var(--fg-dim);text-align:center;padding:20px}
  @media(max-width:760px){.grid{grid-template-columns:1fr}.bookmarks{grid-column:1}.list article{grid-template-columns:1fr auto}.list article>div{grid-column:1/-1}.page{padding:10px}h1{font-size:23px}}
</style>
