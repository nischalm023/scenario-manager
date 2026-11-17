/* Offline charts consume additive counts and observed durations. Rebinning
   always divides summed exposure once; it never averages hourly averages. */
'use strict';
const DATA = JSON.parse(document.querySelector('#report-data').textContent);
const byId = new Map(DATA.cases.map(c => [c.id,c]));
const COLORS = {picks:'#eb6c36',inducted:'#5e7a9b',transfers:'#8a7868',stock:'#7c8f6f',rack:'#5e7a9b',muted:'#8c9198'};
const HOUR = 3600000;
const fmt = (v,d=0) => v == null ? '—' : Number(v).toLocaleString('en-US',{minimumFractionDigits:d,maximumFractionDigits:d});
const esc = v => String(v).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const preloadName = c => ({0:'Empty start',0.5:'Partial preload',1:'Full preload'})[c.preload_fraction];
const timingName = c => c.timing === 't0' ? 'All orders released at T0' : 'Source order timing';
const sequenceName = c => c.sequence === 'sku_demand' ? 'SKU-demand sequence' : c.lead_time_secs ? 'Order-release sequence · 1h lead' : 'Order-release sequence';
const name = c => `${preloadName(c)} · putaway ${c.putaway}`;
const subtitle = c => `${timingName(c)}${c.putaway==='on' ? ' · '+sequenceName(c) : ''}`;
let periodMinutes = 60, horizon = 'whole', scale = 'shared', currentId;

function download(name, text, type){
  const url=URL.createObjectURL(new Blob([text],{type}));
  const a=document.createElement('a');a.href=url;a.download=name;a.click();
  setTimeout(()=>URL.revokeObjectURL(url),1000);
}

function binsFor(series,minutes,until){
  const output=[], width=minutes*60000;
  for(const b of series.bins){
    if(b.start_ms>=until)break;
    if(b.end_ms>until)throw new Error('Display cutoff must coincide with an exported sample boundary');
    const index=Math.floor(b.start_ms/width);
    if(!output[index])output[index]={start_ms:b.start_ms,end_ms:b.end_ms,picks:0,pick_transfers:0,inducted_units:0,filled_totes:0,stored_totes:0,wait_ms:0,implicit_ms:0};
    const row=output[index];row.end_ms=b.end_ms;
    for(const field of ['picks','pick_transfers','inducted_units','filled_totes','stored_totes'])row[field]+=b[field];
    row.wait_ms+=b.wait.elapsed_wait_ms;row.implicit_ms+=b.wait.implicit_hold_ms;
  }
  return output;
}

function rates(bins,fleet){
  return bins.map(b=>{
    const hours=(b.end_ms-b.start_ms)/HOUR;
    return {...b,x:(b.start_ms+b.end_ms)/2/HOUR,hours,pick_rate:b.picks/hours,
      transfer_rate:b.pick_transfers/hours,induction_rate:b.inducted_units/hours,
      filled_rate:b.filled_totes/hours,stored_rate:b.stored_totes/hours,
      wait_rate:b.wait_ms/60000/fleet/hours,implicit_rate:b.implicit_ms/60000/fleet/hours};
  });
}

function chart(containerId,title,unit,rows,lines,until,description,options={}){
  const host=document.getElementById(containerId), w=920,h=320,l=68,r=22,t=20,b=40;
  const xmin=0,xmax=until/HOUR;
  const highest=Math.max(0,...lines.flatMap(s=>rows.map(row=>Number(row[s.key]||0)))) || 1;
  const nice=v=>{const power=10**Math.floor(Math.log10(v));return Math.ceil(v/power*2)/2*power;};
  const ymax=options.ymax||nice(highest*1.06);
  const x=v=>l+(w-l-r)*v/xmax,y=v=>h-b-(h-b-t)*v/ymax;
  let svg=`<svg viewBox="0 0 ${w} ${h}" role="img" tabindex="0" aria-label="${esc(title)}. Use left and right arrows to inspect intervals."><title>${esc(title)}</title><desc>${esc(description)}</desc>`;
  for(let i=0;i<=4;i++){
    const v=i*ymax/4;
    svg+=`<line class="grid" x1="${l}" x2="${w-r}" y1="${y(v)}" y2="${y(v)}"/><text x="${l-10}" y="${y(v)+4}" text-anchor="end">${fmt(v,ymax<10?2:0)}</text>`;
  }
  for(let i=0;i<=8;i++){
    const v=xmax*i/8;
    svg+=`<text x="${x(v)}" y="${h-15}" text-anchor="middle">${fmt(v,xmax%8 ? 1 : 0)}h</text>`;
  }
  for(const line of lines){
    const points=rows.map(row=>`${x(row.x).toFixed(2)},${y(row[line.key]).toFixed(2)}`).join(' ');
    svg+=`<polyline class="trace" points="${points}" stroke="${line.color}"${line.dash?' stroke-dasharray="6 5"':''}/>`;
  }
  svg+=`<line class="cursor" x1="0" x2="0" y1="${t}" y2="${h-b}" visibility="hidden"/></svg>`;
  host.innerHTML=`<div class="chart-head"><h3>${esc(title)}</h3><span>${esc(unit)}</span></div><div class="legend">${lines.map(s=>`<span><i class="swatch" style="--line:${s.color};${s.dash?'border-top-style:dashed':''}"></i>${esc(s.name)}</span>`).join('')}</div><figure>${svg}<figcaption class="caption">${description}</figcaption></figure><div class="readout">Hover the chart, or focus it and use the arrow keys, for values.</div>`;
  const element=host.querySelector('svg'),cursor=host.querySelector('.cursor'),readout=host.querySelector('.readout');
  let selected=0;
  function show(index){
    selected=Math.min(rows.length-1,Math.max(0,index));const row=rows[selected];
    const interval=options.snapshot ? `T=${fmt(row.x,2)}h` : `T=${fmt(row.start_ms/HOUR,2)}–${fmt(row.end_ms/HOUR,2)}h${row.hours<periodMinutes/60-1e-7?' · partial interval':''}`;
    const values=lines.map(line=>`${esc(line.name)}: <strong>${fmt(row[line.key],options.digits??1)}</strong>`).join(' · ');
    readout.innerHTML=`${interval} · ${values}${options.extra?'<br>'+options.extra(row):''}`;
    cursor.setAttribute('x1',x(row.x));cursor.setAttribute('x2',x(row.x));cursor.setAttribute('visibility','visible');
    return `${interval}<br>${values}`;
  }
  element.addEventListener('pointermove',e=>{
    const rect=element.getBoundingClientRect(),at=((e.clientX-rect.left)/rect.width*w-l)/(w-l-r)*xmax;
    const nearest=rows.reduce((best,row,i)=>Math.abs(row.x-at)<Math.abs(rows[best].x-at)?i:best,0);
    show(nearest);
  });
  element.addEventListener('keydown',e=>{if(e.key==='ArrowLeft'||e.key==='ArrowRight'){e.preventDefault();show(selected+(e.key==='ArrowRight'?1:-1));}});
  element.addEventListener('focus',()=>show(selected));
  return {ymax};
}

function sharedMax(kind){
  const maxima=DATA.cases.filter(c=>c.status==='recorded').map(c=>{
    const until=horizon==='common'?Math.min(16*HOUR,c.series.bins.at(-1).end_ms):c.series.bins.at(-1).end_ms;
    const rows=rates(binsFor(c.series,periodMinutes,until),c.series.htm_fleet_size);
    return Math.max(...rows.flatMap(r=>kind==='throughput'?[r.pick_rate,r.induction_rate,r.transfer_rate]:kind==='putaway'?[r.filled_rate,r.stored_rate]:[r.wait_rate]));
  });
  const largest=Math.max(...maxima);
  const step=kind==='wait'?.5:kind==='putaway'?100:500;
  return Math.max(step,Math.ceil(largest/step)*step);
}

function notes(c){
  const items=[];
  if(c.timing==='source')items.push('Only 245 orders are released in the first six simulated hours. Early low pick rates therefore do not measure saturated picking capacity.');
  else items.push('All 22,160 orders are released at T0. This removes customer order-release timing as a restriction, while physical tasks and resource contention remain.');
  if(c.preload_fraction===0)items.push('Cold-start picking depends on the right SKUs arriving through synthetic decant. Six stations each target 60 fills/hour; a fill contains at most six units of one SKU. A low pick rate can reflect unavailable stock and assignment policy even when traffic is light.');
  if(c.preload_fraction===.5)items.push('Partial preload is 50% rounded up separately for every SKU: 747,243 units, or 53.46% of the approved full stock. Half the initially stocked inventory-tote population is used.');
  if(c.preload_fraction===1)items.push('“Full” means the complete approved stock file, not 100% of physical rack capacity and not 100 units in each tote.');
  if(c.putaway==='on')items.push(c.lead_time_secs ? 'Demand lots become eligible one hour before source order release. This is a synthetic rule, not a schedule provided by Walmart. Future priority lots can also delay bulk surplus induction.' : c.sequence==='sku_demand' ? 'Synthetic decant ranks SKUs by total demand. This can supply popular SKUs while specific assigned orders wait for other SKUs.' : 'Synthetic decant follows a static order-release/OrderId sequence. Live assignment falls back to OrderId among released orders; these priorities can diverge.');
  if(c.preload_fraction===0&&c.id.includes('source-order_release')&&!c.lead_time_secs)items.push('The 52-hour run ended with 169 assigned orders waiting on missing stock, while 11,079 unassigned orders were individually stock-ready. That is an assignment/supply mismatch; the available units are not necessarily sufficient to start all those orders simultaneously.');
  items.push('Completed boxes disappear at the sealer handoff in this pass. DS order buffering uses a provisional one-hour hold after storage; SC uses direct extraction. Floor-driving VTM behavior and battery/charging dynamics remain unmodeled.');
  return items.map(text=>`<li>${esc(text)}</li>`).join('');
}

function render(){
  const id=location.hash.slice(1), c=byId.get(id)||byId.get('full-off-t0');currentId=c.id;
  for(const a of document.querySelectorAll('#variants a'))a.setAttribute('aria-current',a.dataset.id===c.id?'page':'false');
  document.title=`Walmart · ${name(c)} · ${timingName(c)}`;
  const index=DATA.cases.indexOf(c),related=DATA.cases;
  const pager=`<div class="pager">${index?`<a href="#${related[index-1].id}">← Previous variant</a>`:'<span></span>'}${index<related.length-1?`<a href="#${related[index+1].id}">Next variant →</a>`:''}</div>`;
  const status=c.status==='preflight'?'Stock preflight rejected':c.status==='equivalent'?'Equivalent configuration · no independent run':'Recorded and audited';
  let out=`<span class="eyebrow">Variant ${index+1} / ${DATA.cases.length} · iteration ${c.iteration} · ${status}</span><h2>${name(c)}</h2><p class="subtitle">${subtitle(c)}</p><p class="small"><code>${c.id}</code></p>
  <div class="chips"><span class="chip">230 HTMs</span><span class="chip">40 inventory + 8 buffer VTMs</span><span class="chip">4 EasyLoops · 10s / transfer</span><span class="chip">1h order-buffer hold</span><span class="chip">22,160 orders</span></div>`;
  if(c.status==='preflight'){
    out+=`<div class="notice"><p><strong>This variant did not simulate.</strong> With putaway disabled, its initial stock cannot supply all orders: ${fmt(c.preload.missing_units)} units are missing across ${fmt(c.preload.missing_skus)} SKUs.</p><p>The stock preflight correctly stopped the run. There is no recorded throughput, inventory history or traffic history to plot; these are unavailable values, not measured zeros.</p></div><div class="empty-charts">${['Throughput','Inventory fill','Navigational waiting'].map(label=>`<div class="unavailable"><h3>${label}</h3><p>No recording: rejected before simulation.</p></div>`).join('')}</div><p class="small" style="margin-top:18px">Configured preload: ${fmt(c.preload.preloaded_units)} units. Source order timing does not repair a total stock deficit when there is no inbound supply.</p>`;
  }else{
    const source=c.status==='equivalent'?byId.get(c.alias_of):c, s=source.series;
    const available=c.status==='equivalent'?Math.min(c.hours*HOUR,s.bins.at(-1).end_ms):s.bins.at(-1).end_ms;
    const until=horizon==='common'?Math.min(16*HOUR,available):available;
    if(c.status==='equivalent')out+=`<div class="notice">All approved inventory is preloaded, leaving no finite inbound workload. The charts below reuse <a href="#${source.id}">${esc(source.id)}</a>, restricted to this variant’s ${c.hours}h horizon. They are not a second measurement.</div>`;
    out+=`<p class="small">Observed recording: ${fmt(s.recorded_hours,2)} simulated hours${c.status==='equivalent'?' (source recording)':''}. Use the first 16 hours for comparisons across variants. This companion uses the shared navigational-wait filter; it excludes all EasyLoop cells, more broadly than the original report.</p>
    <div class="toolbar"><label>Interval<select id="period"><option value="15" ${periodMinutes===15?'selected':''}>15 minutes</option><option value="60" ${periodMinutes===60?'selected':''}>1 hour</option></select></label><label>Time window<select id="horizon"><option value="whole" ${horizon==='whole'?'selected':''}>Whole observed run</option><option value="common" ${horizon==='common'?'selected':''}>First 16 hours</option></select></label><label>Rate / wait scales<select id="scale"><option value="shared" ${scale==='shared'?'selected':''}>Same across variants</option><option value="local" ${scale==='local'?'selected':''}>Fit this variant</option></select></label><button id="csv">Download this page’s CSV</button></div><div id="kpis" class="kpis"></div><section id="throughput" class="chart"></section><section id="putaway" class="chart"></section><section id="inventory" class="chart"></section><section id="waiting" class="chart"></section><details><summary>Interval values and denominators</summary><div id="values" class="table-wrap"></div></details>`;
  }
  out+=`<details open><summary>How to interpret this variant</summary><div><ul>${notes(c)}</ul></div></details><details><summary>Measurement definitions, shared filter and provenance</summary><div><p><strong>Throughput:</strong> completed pick units and inducted inventory units from timestamped journals. Filled inventory totes and completed VTM rack stores are separate counts. The 1,440/hour operator bound is for transfer tasks across four operators, not necessarily units.</p><p><strong>Inventory units:</strong> initial source inventory + completed induction units − completed pick units. Includes stock in racks and in circulation; excludes units already transferred into order containers. Percentages use the approved 1,397,632-unit full inventory as a reference, not a warehouse unit-capacity limit.</p><p><strong>Rack fill:</strong> physical, timestamped source-rack snapshots. “Stocked” means a slot containing a tote with at least one unit; “occupied” includes empty totes. Source capacity is 170,570 slots; the loader’s 174,020-slot total also includes 3,450 order-storage slots. Order-buffer/container storage is excluded from this plot.</p><p><strong>Navigational waiting:</strong> sum of elapsed HTM waiting in eligible cells ÷ 230 HTMs ÷ observed simulated hours, shown in minutes per HTM per hour. For example, 2 minutes means 3.33% of fleet time. All fleet HTMs are in the denominator, including idle ones. Rotation is separate and is not counted as waiting. Both explicit wait steps and implicit stationary holds are counted.</p><p>The shared <code>WarehouseMap::navigation_wait_exclusions</code> filter excludes typed queue, presentation, exchange, relay and charging cells; relay access cells; station presentation and queue cells; declared handoff queues; inventory holding queues; and complete conveyor-equipment membership, including both EasyLoop waiting blocks and circulation lanes. An upstream ordinary cell remains included if a robot waits there. This is a location filter, not causal proof of congestion.</p><p>Plots use additive five-minute exports, rebinned to the selected interval. Counts are divided by actual interval duration. Inventory is sampled at interval endpoints. Partial final intervals are labelled; “peak full-hour” excludes them. Waits are clipped at the observation boundary, including ongoing waits. Re-extracting these profiles does not rerun the simulator.</p><p>Frozen simulation source: <code>${DATA.manifest.source_commit}</code>. Frozen binary SHA-256: <code>${DATA.manifest.binary_sha256}</code>. The recorded runtime Git label may differ; the frozen binary and manifest are the provenance for this experiment.</p><p>Original report SHA-256 (unchanged): <code>${DATA.original_report_sha256}</code>.</p>${c.series?`<p>Profile: <code>${esc(c.series.profile)}</code><br>SHA-256: <code>${c.series.profile_sha256}</code><br>${fmt(c.series.excluded_cell_ids.length)} cells excluded by the shared filter, including ${c.series.equipment_cell_ids.length} conveyor-equipment cells. Completion totals and final inventory reconcile with the prior independent flow audits; bucket waits reconcile with cumulative elapsed traffic.</p>`:''}</div></details><details><summary>Exact scenario configuration</summary><div><pre>${esc(JSON.stringify(c.config,null,2))}</pre></div></details>${pager}<footer>Companion generated ${esc(DATA.generated_at)}. Self-contained HTML; no network access or simulation files required to view it. <a href="report.html">Original matrix report</a> is a separate document.</footer>`;
  document.querySelector('main').innerHTML=out;
  if(c.status==='preflight')return;
  const source=c.status==='equivalent'?byId.get(c.alias_of):c,s=source.series;
  const available=c.status==='equivalent'?Math.min(c.hours*HOUR,s.bins.at(-1).end_ms):s.bins.at(-1).end_ms;
  const until=horizon==='common'?Math.min(16*HOUR,available):available;
  const rows=rates(binsFor(s,periodMinutes,until),s.htm_fleet_size), hours=until/HOUR;
  const sum=key=>rows.reduce((n,r)=>n+r[key],0);
  const rackAt=new Map(s.racks.map(r=>[r.at_ms,r]));
  const inventory=[s.racks[0],...rows.map(r=>rackAt.get(r.end_ms))].map(r=>({...r,x:r.at_ms/HOUR,units:s.initial_units+r.net_inventory_units,unit_pct:(s.initial_units+r.net_inventory_units)/DATA.source.preload.full.preloaded_units*100,stocked_pct:r.stocked/r.capacity*100,occupied_pct:r.occupied/r.capacity*100}));
  const last=inventory.at(-1),hourRows=rates(binsFor(s,60,until),s.htm_fleet_size);
  const peak=Math.max(0,...hourRows.filter(r=>r.hours>=1-1e-9).map(r=>r.pick_rate));
  const kpis=[['Pick units / hour',sum('picks')/hours,1,`${fmt(sum('picks'))} units in ${fmt(hours,2)}h`],['Peak complete hour',peak,0,'Pick units; excludes partial final hour'],['Inducted units / hour',sum('inducted_units')/hours,1,`${fmt(sum('filled_totes'))} filled inventory totes`],['Stored totes / hour',sum('stored_totes')/hours,1,'Completed filled-tote rack deposits'],['Source inventory at end',last.units,0,`${fmt(last.unit_pct,2)}% of full-stock reference`],['Wait minutes / HTM / hour',sum('wait_ms')/60000/s.htm_fleet_size/hours,3,`Ordinary navigation · ${fmt(sum('wait_ms')/HOUR,2)} total HTM-hours`]];
  document.querySelector('#kpis').innerHTML=kpis.map(([label,value,digits,detail])=>`<div class="kpi"><span>${label}</span><strong>${fmt(value,digits)}</strong><span>${detail}</span></div>`).join('');
  const limit=kind=>scale==='shared'?sharedMax(kind):undefined;
  chart('throughput','Pick and inventory induction throughput','units or transfer tasks / hour',rows,[{key:'pick_rate',name:'Pick units',color:COLORS.picks},{key:'induction_rate',name:'Inducted inventory units',color:COLORS.inducted},{key:'transfer_rate',name:'Pick transfer tasks',color:COLORS.transfers,dash:true}],until,`Rates over each ${periodMinutes}-minute interval, plotted at its midpoint. A transfer can move multiple units. Four operators provide a zero-loss bound of 1,440 transfer tasks/hour.`,{ymax:limit('throughput'),extra:r=>`${fmt(r.picks)} pick units · ${fmt(r.inducted_units)} inducted units · ${fmt(r.hours,3)}h observed`});
  chart('putaway','Putaway flow through decant and rack storage','inventory totes / hour',rows,[{key:'filled_rate',name:'Totes filled at decant',color:COLORS.inducted},{key:'stored_rate',name:'Filled totes stored in racks',color:COLORS.stock}],until,'Filling a tote and storing it are different milestones. Their curves can differ because an HTM/VTM transport leg remains after filling.',{ymax:limit('putaway')});
  chart('inventory','Inventory fill','percent of the stated denominator',inventory,[{key:'unit_pct',name:'Source units / full-stock reference',color:COLORS.stock},{key:'stocked_pct',name:'Source rack slots with stock',color:COLORS.rack},{key:'occupied_pct',name:'Source rack slots with any tote',color:COLORS.muted,dash:true}],until,`Source rack capacity: ${fmt(last.capacity)} slots; order storage is excluded. Empty totes already occupy racks at startup. Unit stock includes inventory in circulation. Start → end units: ${fmt(s.initial_units)} → ${fmt(last.units)} (${fmt(last.units-s.initial_units)} net).`,{snapshot:true,ymax:100,digits:3,extra:r=>`${fmt(r.units)} source units · ${fmt(r.stocked)} stocked slots · ${fmt(r.empty)} empty-tote slots · ${fmt(r.capacity-r.occupied)} unoccupied slots`});
  chart('waiting','Average HTM waiting in navigation cells','minutes / HTM / simulated hour',rows,[{key:'wait_rate',name:'All eligible waiting',color:COLORS.picks},{key:'implicit_rate',name:'Implicit stationary holds (included above)',color:COLORS.muted,dash:true}],until,`Total eligible HTM wait ÷ ${s.htm_fleet_size} HTMs ÷ observed interval. Queue, presentation, access, charging and all EasyLoop cells are excluded by the shared filter. This includes task/resource waiting on ordinary cells; it is not all attributable to traffic congestion.`,{ymax:limit('wait'),digits:3,extra:r=>`${fmt(r.wait_ms/60000,2)} HTM-minutes / (${s.htm_fleet_size} HTMs × ${fmt(r.hours,3)}h) · ${fmt(r.wait_rate/60*100,3)}% of fleet time`});
  document.querySelector('#values').innerHTML=`<table><thead><tr><th>Sim hours</th><th>Pick units/h</th><th>Inducted units/h</th><th>Rack stores/h</th><th>Source units at end</th><th>Stocked slots %</th><th>Wait min/HTM/h</th><th>Observed hours</th></tr></thead><tbody>${rows.map((r,i)=>`<tr><td>${fmt(r.start_ms/HOUR,2)}–${fmt(r.end_ms/HOUR,2)}</td><td>${fmt(r.pick_rate,1)}</td><td>${fmt(r.induction_rate,1)}</td><td>${fmt(r.stored_rate,1)}</td><td>${fmt(inventory[i+1].units)}</td><td>${fmt(inventory[i+1].stocked_pct,3)}</td><td>${fmt(r.wait_rate,4)}</td><td>${fmt(r.hours,4)}</td></tr>`).join('')}</tbody></table>`;
  for(const [id,setter] of [['period',v=>periodMinutes=Number(v)],['horizon',v=>horizon=v],['scale',v=>scale=v]])document.getElementById(id).addEventListener('change',e=>{setter(e.target.value);render();});
  document.getElementById('csv').addEventListener('click',()=>{
    const lines=['variant,source_recording,start_hours,end_hours,observed_hours,pick_units,pick_units_per_hour,pick_transfer_tasks,inducted_units,inducted_units_per_hour,filled_totes,stored_totes,source_units_end,source_stocked_slots,source_capacity_slots,eligible_htm_wait_minutes,htms,wait_minutes_per_htm_per_hour'];
    rows.forEach((r,i)=>lines.push([c.id,source.id,r.start_ms/HOUR,r.end_ms/HOUR,r.hours,r.picks,r.pick_rate,r.pick_transfers,r.inducted_units,r.induction_rate,r.filled_totes,r.stored_totes,inventory[i+1].units,inventory[i+1].stocked,inventory[i+1].capacity,r.wait_ms/60000,s.htm_fleet_size,r.wait_rate].join(',')));
    download(c.id+'-time-series.csv',lines.join('\n'),'text/csv');
  });
}

function buildNav(){
  const q=document.getElementById('search').value.trim().toLowerCase();
  let out='';
  for(const fraction of [1,.5,0]){
    const cases=DATA.cases.filter(c=>c.preload_fraction===fraction&&(`${c.id} ${name(c)} ${subtitle(c)}`).toLowerCase().includes(q));
    if(!cases.length)continue;
    out+=`<p class="eyebrow group">${preloadName(cases[0])}</p>`;
    out+=cases.map(c=>`<a href="#${c.id}" data-id="${c.id}" aria-current="${c.id===currentId?'page':'false'}">Putaway ${c.putaway} · ${c.timing==='t0'?'T0 orders':'source timing'}<span class="mark ${c.status}">${c.status==='recorded'?'run':c.status==='preflight'?'rejected':'equiv.'}</span><small>${c.putaway==='on'?sequenceName(c):'No induction'} · iteration ${c.iteration}</small></a>`).join('');
  }
  document.getElementById('variants').innerHTML=out||'<p class="small">No variants match.</p>';
}
document.getElementById('search').addEventListener('input',buildNav);
document.getElementById('download').addEventListener('click',()=>download('walmart-variant-chart-data.json',JSON.stringify(DATA,null,2),'application/json'));
window.addEventListener('hashchange',()=>{render();document.querySelector('main').focus({preventScroll:true});window.scrollTo({top:0});});
buildNav();render();
