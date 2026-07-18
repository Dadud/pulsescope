<script lang="ts">
  import type { PositionEvent } from './api';
  let { points, selected, onselect }:{points:PositionEvent[],selected?:number,onselect:(p:PositionEvent)=>void}=$props();
  const x=(lon:number)=>(lon+180)/360*1000, y=(lat:number)=>(90-lat)/180*500;
  let trails=$derived(Object.values(points.reduce((a,p)=>{(a[p.entity_id]??=[]).push(p);return a},{} as Record<string,PositionEvent[]>)));
</script>
<div class="map" aria-label="Offline coordinate map">
  <svg viewBox="0 0 1000 500" role="img">
    <rect width="1000" height="500" class="ocean"/>
    {#each [-120,-60,0,60,120] as lon}<line x1={x(lon)} x2={x(lon)} y1="0" y2="500"/>{/each}
    {#each [-60,-30,0,30,60] as lat}<line x1="0" x2="1000" y1={y(lat)} y2={y(lat)}/>{/each}
    {#each trails as trail}
      <polyline points={trail.map(p=>`${x(p.longitude)},${y(p.latitude)}`).join(' ')} />
    {/each}
    {#each points as p,i (p.id ?? `${p.entity_id}-${p.timestamp_ms}`)}
      <circle role="button" tabindex="0" aria-label={`Select ${p.entity_id}`} class:selected={selected===i} cx={x(p.longitude)} cy={y(p.latitude)} r={selected===i?7:4} onclick={()=>onselect(p)} onkeydown={(e)=>['Enter',' '].includes(e.key)&&onselect(p)}><title>{p.entity_id} · {p.source}</title></circle>
    {/each}
  </svg>
  <small>Local projection · no tiles or coordinates are sent over the network</small>
</div>
<style>.map{background:#09131c;border:1px solid var(--line);border-radius:6px;overflow:hidden}.map svg{display:block;width:100%;max-height:55vh}.ocean{fill:#0b1b27}line{stroke:#29404e;stroke-width:.6}polyline{fill:none;stroke:#38a3c7;stroke-width:1.5;opacity:.55}circle{fill:#ffcc4d;stroke:#111;cursor:pointer}circle.selected{fill:#ff675c}small{display:block;padding:5px 8px;color:var(--fg-dim)}</style>
