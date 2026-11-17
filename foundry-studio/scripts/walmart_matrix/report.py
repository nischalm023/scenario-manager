#!/usr/bin/env python3
"""Build a self-contained, evidence-linked morning report from matrix artifacts.

Pending cases never acquire invented rates. Every refresh embeds its data and
timestamp, so the HTML remains useful when sent without this repository.
"""
import argparse
from collections import Counter
import datetime as dt
import html
import json
import math
import os
from pathlib import Path
import re

from run import BASE, REPO, now

DEST=REPO/'docs/easyloop/overnight-2026-09-14/report.html'
PALETTE=['#eb6c36','#5e7a9b','#7c8f6f','#b8915a']


def esc(value):return html.escape(str(value))
def read(path,default=None):
    return json.loads(path.read_text()) if path.exists() else default
def num(value,digits=0):return '—' if value is None else f'{value:,.{digits}f}'


def progress(folder):
    path=folder/'run.log'
    if not path.exists():return ''
    with path.open('rb') as f:
        f.seek(max(0,path.stat().st_size-3000));s=f.read().decode(errors='replace')
    s=re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]','',s).replace('\r','\n')
    rows=[row for row in s.splitlines() if row.strip()]
    return next((row.strip() for row in reversed(rows) if ' picks ' in row),'')


def plot(slug,title,series,description,bin_hours=2,unit='units/hour'):
    data=[]
    for name,values in series:
        if not values:continue
        points=[]
        for start in range(0,len(values),bin_hours):
            chunk=values[start:start+bin_hours]
            if len(chunk)==bin_hours:points.append((start+bin_hours/2,sum(chunk)/bin_hours))
        if points:data.append((name,points))
    if not data:return f'<p class="pending">{esc(title)}: awaiting completed recordings.</p>'
    xmax=max(points[-1][0]+bin_hours/2 for _,points in data)
    ymax=max(100,math.ceil(max(y for _,pts in data for x,y in pts)/250)*250)
    x=lambda t:80+860*t/xmax
    y=lambda v:356-280*v/ymax
    out=[f'<h3>{esc(title)}</h3><figure><svg viewBox="0 0 1000 456" role="img" aria-labelledby="{slug}-title {slug}-desc"><title id="{slug}-title">{esc(title)}</title><desc id="{slug}-desc">{esc(description)}</desc>']
    for i in range(5):
        val=ymax*i/4
        out.append(f'<line x1="80" y1="{y(val):.2f}" x2="940" y2="{y(val):.2f}" stroke="#d9dadc"/><text x="68" y="{y(val)+4:.2f}" text-anchor="end">{num(val)}</text>')
    for t in range(0,int(xmax)+1,max(bin_hours,int(xmax//8/bin_hours)*bin_hours)):
        out.append(f'<text x="{x(t):.2f}" y="380" text-anchor="middle">{t}h</text>')
    out.append(f'<text x="80" y="48">{esc(unit)} · {bin_hours}-hour averages</text>')
    for j,(name,points) in enumerate(data):
        pts=' '.join(f'{x(a):.2f},{y(b):.2f}' for a,b in points)
        out.append(f'<polyline points="{pts}" fill="none" stroke="{PALETTE[j]}" stroke-width="3" stroke-linejoin="round"/>')
        for a,b in points:
            out.append(f'<circle cx="{x(a):.2f}" cy="{y(b):.2f}" r="3" fill="{PALETTE[j]}"><title>T={a-bin_hours/2:g}–{a+bin_hours/2:g}h: {b:.1f} {esc(unit)}</title></circle>')
        out.append(f'<rect x="{80+j*284}" y="410" width="16" height="8" fill="{PALETTE[j]}"/><text x="{104+j*284}" y="419">{esc(name)}</text>')
    out.append(f'</svg><figcaption>{esc(description)}</figcaption></figure>')
    return ''.join(out)


def traffic_figure(case,traffic):
    eligible=[row for row in traffic['cells'] if row['exclusion'] is None and row['x_mm'] is not None]
    top=sorted(eligible,key=lambda r:r['wait_s'],reverse=True)[:120]
    top=[row for row in top if row['wait_s']>0]
    if not top:return '<p>No stationary waiting recorded in eligible travel cells.</p>'
    floors=traffic['floors']
    xmin=min(f['rect_mm'][0] for f in floors);xmax=max(f['rect_mm'][0]+f['rect_mm'][2] for f in floors)
    ymin=min(f['rect_mm'][1] for f in floors);ymax=max(f['rect_mm'][1]+f['rect_mm'][3] for f in floors)
    scale=min(940/(xmax-xmin),540/(ymax-ymin))
    x=lambda v:28+(v-xmin)*scale
    y=lambda v:28+(v-ymin)*scale
    slug='traffic-'+case['id']
    out=[f'<figure><svg viewBox="0 0 1000 620" role="img" aria-labelledby="{slug}-title {slug}-desc"><title id="{slug}-title">Travel-cell waiting hotspots</title><desc id="{slug}-desc">Top 120 eligible cells by cumulative stationary waiting; intentional presentation and queue cells are excluded.</desc>']
    for floor in floors:
        a,b,w,h=floor['rect_mm']
        out.append(f'<rect x="{x(a):.1f}" y="{y(b):.1f}" width="{w*scale:.1f}" height="{h*scale:.1f}" fill="#e1e3e5"/>')
    maximum=top[0]['wait_s']
    for row in reversed(top):
        radius=2.5+5*math.sqrt(row['wait_s']/maximum)
        out.append(f'<circle cx="{x(row["x_mm"]):.1f}" cy="{y(row["y_mm"]):.1f}" r="{radius:.1f}" fill="#eb6c36" fill-opacity=".75"><title>Cell {row["cell"]}: {row["wait_s"]:.1f} HTM-seconds wait; moving mean {num(row["mean_m_s"],2)} m/s; ({row["x_mm"]/1000:.1f}, {row["y_mm"]/1000:.1f}) m</title></circle>')
    out.append('</svg><figcaption>Larger circles indicate more accumulated waiting. Only the top 120 eligible cells are shown; unmarked cells are not necessarily unobserved or wait-free. Coordinates preserve the simulator map orientation.</figcaption></figure>')
    return ''.join(out)


def speed_figure(case,bins):
    total=sum(bins)
    if not total:return ''
    maximum=max(bins)/total*100
    slug='speed-'+case['id']
    out=[f'<figure><svg viewBox="0 0 1000 330" role="img" aria-labelledby="{slug}-title {slug}-desc"><title id="{slug}-title">Moving-speed distribution</title><desc id="{slug}-desc">Share of eligible travel-cell moving time in half-metre-per-second speed bins. Stops and turns in place are excluded.</desc>']
    for i,seconds in enumerate(bins):
        share=seconds/total*100;height=200*share/maximum;x=65+i*81
        label=f'{i*.5:g}–{(i+1)*.5:g}' if i<10 else '≥5'
        out.append(f'<rect x="{x}" y="{250-height:.1f}" width="54" height="{height:.1f}" fill="#5e7a9b"><title>{label} m/s: {share:.2f}% of moving time ({seconds:,.1f} HTM-seconds)</title></rect><text x="{x+27}" y="{239-height:.1f}" text-anchor="middle">{share:.1f}%</text><text x="{x+27}" y="278" text-anchor="middle">{label}</text>')
    out.append('<text x="65" y="312">m/s · duration-weighted · hover bars for exposure</text></svg><figcaption>Movement only. A whole-run mixture of slow and fast motion cannot establish whether crossing traffic delayed a faster traffic wave.</figcaption></figure>')
    return ''.join(out)


def main(base=BASE, output=DEST):
    manifest=read(base/'manifest.json');source=read(base/'source-audit.json',{})
    cases=[]
    for case in manifest['cases']:
        folder=base/case['id'];c=dict(case);c['state']=read(folder/'state.json',{});c['facts']=read(folder/'report-facts.json')
        c['flow']=read(folder/'flow-audit.json');c['buffer']=read(folder/'buffer-audit.json')
        c['backlog']=read(folder/'backlog.json');c['backlog_error']=read(folder/'backlog-error.json')
        c['traffic_path']=folder/'traffic.json';c['progress']=progress(folder)
        c['analysis_error']=read(folder/'analysis-error.json')
        c['stock_preflight']=False
        if c['state'].get('status')=='failed' and (folder/'run.log').exists():
            with (folder/'run.log').open('rb') as log:
                c['stock_preflight']=b'Inventory cannot fulfill all orders' in log.read(1_000_000)
        cases.append(c)
    by_id={c['id']:c for c in cases}
    actual=[c for c in cases if c['state'].get('status')!='equivalent_no_inbound']
    audited=[c for c in actual if c['facts']]
    waiting=[c for c in actual if c['state'].get('status') in ('running','queued')]
    rejected=[c for c in actual if c['stock_preflight']]
    awaiting_audit=[c for c in actual if c['state'].get('status')=='finished' and not c['facts'] and not c['analysis_error']]
    failures=[c for c in actual if (c['state'].get('status')=='failed' and c not in rejected) or c['analysis_error'] or (c['facts'] and any(c['facts']['audit_exit_codes'].values()))]
    complete=not waiting and not awaiting_audit
    status=('Runs finished · inspect validation outcomes' if failures else 'Completed experiments') if complete else 'Overnight experiments in progress'
    rows=[]
    for c in cases:
        state=c['state'].get('status');facts=c['facts'];w=facts['windows']['common'] if facts else None
        if state=='equivalent_no_inbound':label='Equivalent: no inbound stock'
        elif state=='failed':label='Preflight rejected' if c['stock_preflight'] else 'Run failed'
        elif c['analysis_error']:label='Analysis failed'
        elif facts:
            label='Audited' if not any(facts['audit_exit_codes'].values()) else 'Audit discrepancy'
            if w['hours']<16:label+=f' · stopped at {w["hours"]:.2f}h'
        else:label=('Awaiting audit' if state=='finished' else state.replace('_',' ').capitalize()) if state else 'Unknown'
        preload={0.:'Empty',.5:'Partial',1.:'Full'}[c['preload_fraction']]
        timing='Source drops' if c['timing']=='source' else 'All at T0'
        induction='Off' if c['putaway']=='off' else ('Demand-ranked' if c['sequence']=='sku_demand' else 'Order-priority')
        if c['lead_time_secs'] is not None:induction+=' · 1h lead'
        rate=lambda key:sum(w[key].values())/w['hours'] if w else None
        rows.append(f'<tr><td><a href="#{c["id"]}">{preload}</a></td><td>{timing}</td><td>{induction}</td><td>{esc(label)}</td><td class="n">{num(w["picks_per_hour"] if w else None,1)}</td><td class="n">{num(w["peak_full_hour_picks"] if w else None)}</td><td class="n">{num(rate("inducted_units_by_hour"),1)}</td><td class="n">{num(rate("stored_totes_by_hour"),1)}</td></tr>')
    def series(ids,key='hourly_picks',window='common'):
        answer=[]
        for name,id in ids:
            c=by_id[id]
            if c['facts']:
                w=c['facts']['windows'][window]
                answer.append((name,[v for k,v in sorted(w[key].items(),key=lambda x:int(x[0]))
                                    if int(k)<w['full_hours']]))
        return answer
    charts=plot('t0-picks','Picking with all orders available',series([
        ('Full · putaway off','full-off-t0'),('Partial · putaway on','partial-on-t0-order_release'),('Empty · putaway on','empty-on-t0-order_release')]),
        'Same 16-hour window, order-priority synthetic induction where enabled. Tables retain one-hour peaks; chart points average two hours.')
    charts+=plot('timed-picks','Picking with source order timing',series([
        ('Full · putaway off','full-off-source'),('Partial · putaway on','partial-on-source-order_release'),('Empty · putaway on','empty-on-source-order_release')]),
        'Source-timed orders, first 16 hours. Low rates before sufficient order release are demand-limited, not necessarily a transport bottleneck.')
    charts+=plot('inbound-units','Inducted units with all orders available',series([
        ('Partial · order-priority','partial-on-t0-order_release'),('Empty · order-priority','empty-on-t0-order_release'),('Empty · demand-ranked','empty-on-t0-sku_demand')],'inducted_units_by_hour'),
        'Units physically placed in source totes at decant completion. A completed rack store occurs later; it has its own rate.')
    charts+=plot('empty-timed-policy','Empty start: induction policy with source order timing',series([
        ('Order-priority · eligible T0','empty-on-source-order_release'),
        ('Order-priority · 1h lead','empty-on-source-order_release-lead1h'),
        ('Demand-ranked · eligible T0','empty-on-source-sku_demand')]),
        'First 16 hours. The one-hour lead changes inbound eligibility as well as inventory accumulation; it is synthetic, not a supplied receiving schedule.')
    charts+=plot('partial-timed-policy','Partial preload: induction policy with source order timing',series([
        ('Order-priority · eligible T0','partial-on-source-order_release'),
        ('Order-priority · 1h lead','partial-on-source-order_release-lead1h'),
        ('Demand-ranked · eligible T0','partial-on-source-sku_demand')]),
        'Most demanded stock is already preloaded. The remaining inbound sequence and release timing still change replenishment work and resource use.')
    source_values=[source.get('hourly',{}).get(str(h),{}).get('units',0) for h in range(48)]
    source_chart=plot('source-arrivals','Demand released by the source order file',[('Released units',source_values)],
        'Elapsed time begins at the earliest source drop. Four-hour averages summarize the full 47.28-hour drop span; exact hourly counts are below.',bin_hours=4)
    extended=plot('extended-source','Extended source-timed runs',series([
        ('Full · putaway off','full-off-source'),('Empty · order-priority','empty-on-source-order_release')],window='whole'),
        'Four-hour averages use complete bins only; any final partial bin is omitted. Actual stop times and totals appear below. A time limit does not imply that all orders were fulfilled.',bin_hours=4)
    first6=sum(source.get('hourly',{}).get(str(h),{}).get('orders',0) for h in range(6))
    hourly_source=''.join(f'<tr><td>{h}–{int(h)+1}h</td><td class="n">{num(r["orders"])}</td><td class="n">{num(r["units"])}</td></tr>' for h,r in source.get('hourly',{}).items())
    details=[]
    for c in cases:
        folder=base/c['id'];facts=c['facts'];preload={0.:'Empty',.5:'Partial',1.:'Full'}[c['preload_fraction']]
        section=[f'<section id="{c["id"]}" class="case"><h3>Iteration {c["iteration"]}: {esc(c["id"])}</h3>']
        section.append(f'<p>{preload} preload · putaway {c["putaway"]} · {"original order drop times" if c["timing"]=="source" else "all orders released at T0"} · requested {c["hours"]}h.</p>')
        if c['state'].get('finished_at') and c['state'].get('started_at'):
            elapsed=(dt.datetime.fromisoformat(c['state']['finished_at'])-dt.datetime.fromisoformat(c['state']['started_at'])).total_seconds()/60
            rss=c['state'].get('peak_sampled_rss_mb')
            section.append(f'<p class="source">Execution: {elapsed:.1f} wall-clock minutes; peak sampled resident memory {num(rss/1024 if rss is not None else None,2)} GiB. Samples can miss shorter memory peaks.</p>')
        if c['state'].get('status')=='equivalent_no_inbound':
            section.append(f'<p class="note">All finite stock is preloaded. The synthetic inbound queue is empty, so this combination has no putaway traffic. Its physical workload is equivalent to <a href="#{c["alias_of"]}">{esc(c["alias_of"])}</a>; it was not counted as an independently executed experiment.</p>')
        elif facts:
            w=facts['windows']['whole'];common=facts['windows']['common'];hours=w['hours']
            section.append(f'<p><strong>{num(w["picks"])} units picked in {hours:.2f}h</strong> ({w["picks_per_hour"]:.1f}/h overall). Peak full hour: {num(w["peak_full_hour_picks"])} at T={w["peak_hour_start"]}–{w["peak_hour_start"]+1}h. Best rolling 60-minute count: {num(w["rolling_hour_peak"])}. {num(w["multiunit_transfer_events"])} transfer events moved more than one unit.</p>')
            section.append(f'<p>Reported fleet idle: HTM {facts["summary_idle_pct"]["htm"]:.1f}%, VTM {facts["summary_idle_pct"]["vtm"]:.1f}%. First-16h idle from integrated duty intervals: HTM {common["duty_htm_idle_pct"]:.1f}%, VTM {common["duty_vtm_idle_pct"]:.1f}%. Idle is time outside tracked busy states, not all stationary time or proof that an HTM is unladen and immediately reusable. Instantaneous state-count snapshots are not used to estimate fleet-time shares.</p>')
            section.append(f'<p>{num(w["transfer_events"])} completed transfer tasks ({w["transfer_events"]/hours:.1f}/h), or {100*w["transfer_events"]/hours/1440:.1f}% of the four-operator, ten-second service bound. This measures completed service against the bound; it excludes any partially completed service at the endpoint. Picked units can exceed the transfer count when one task moves several units.</p>')
            columns=['hourly_picks']+(['hourly_transfers'] if 'hourly_transfers' in w else [])+['inducted_units_by_hour','filled_totes_by_hour','stored_totes_by_hour']
            tr=''.join('<tr><td>'+f'{h}–{round(min(int(h)+1,hours),2):g}h'+'</td>'+''.join(f'<td class="n">{num(w[key].get(h,0))}</td>' for key in columns)+'</tr>' for h in w['hourly_picks'])
            transfer_heading='<th>Transfer tasks</th>' if 'hourly_transfers' in w else ''
            section.append('<details><summary>Hourly physical completions</summary><div class="scroll"><table><thead><tr><th>Interval</th><th>Picked units</th>'+transfer_heading+'<th>Inducted units</th><th>Filled totes</th><th>Rack stores</th></tr></thead><tbody>'+tr+'</tbody></table></div></details>')
            flow=c['flow'] or {};codes=facts['audit_exit_codes']
            journals=flow.get('flow_journal_counts') or {}
            section.append(f'<p>Source inventory from completed work: {num(flow.get("initial_inventory"))} initial + {num(journals.get("putaways_completed"))} inducted − {num(journals.get("picks_completed"))} picked = {num(flow.get("expected_inventory"))} expected. Inventory snapshot after excluding timestamped future work: {num(flow.get("final_inventory"))}; raw final bucket: {num(flow.get("raw_final_inventory",flow.get("final_inventory")))}. This balance excludes units already transferred into order containers. Audit exit codes: {esc(codes)} (zero means passed).</p>')
            if flow.get('extraction_journal_checked'):
                section.append('<p>Extraction completion counts and extracted-unit totals also reconcile against the independent timestamped extraction stream.</p>')
            if flow.get('errors'):section.append('<pre>'+esc(json.dumps(flow['errors'],indent=2))+'</pre>')
            if c['backlog']:
                diagnosis=c['backlog'];trs=[]
                for snapshot in diagnosis['snapshots']:
                    trs.append(f'<tr><td>{num(snapshot["hours"],1)}h</td><td class="n">{num(snapshot["assigned_waiting_for_stock"])}</td><td class="n">{num(snapshot["waiting_for_never_inducted_sku"])}</td><td class="n">{num(snapshot["median_assignment_wait_hours"],2)}</td><td class="n">{num(snapshot["released_unassigned"])}</td><td class="n">{num(snapshot["individually_stock_ready_unassigned"])}</td></tr>')
                section.append('<details><summary>Assigned orders versus available inventory</summary><p>Unreserved supply is reconstructed from preload and timestamped induction, less all units reserved when order preparation starts. A stock-ready unassigned order is individually feasible; these candidates may compete for the same units.</p><div class="scroll"><table><thead><tr><th>Time</th><th>Assigned, not prepared</th><th>Need SKU never inducted</th><th>Median wait (h)</th><th>Released, unassigned</th><th>Stock-ready among unassigned</th></tr></thead><tbody>'+''.join(trs)+'</tbody></table></div></details>')
                if diagnosis['post_cutoff_fills']:
                    excess=sum(r['quantity'] for r in diagnosis['post_cutoff_fills'])
                    section.append(f'<p class="note">The recording retains {len(diagnosis["post_cutoff_fills"])} accepted fill(s), totaling {excess} units, after the observation cutoff. Throughput excludes them. Raw time buckets can include future completions; the corrected audit removes only journal-confirmed events after the cutoff in the boundary bucket, while retaining events exactly at the cutoff. Raw values and all corrections remain in the audit JSON.</p>')
            if c['backlog_error']:section.append(f'<p class="note">Backlog diagnosis failed: {esc(c["backlog_error"])}</p>')
            if c['buffer']:
                b=c['buffer'];residence=b.get('residence_completed') or {}
                release=b.get('eligibility_to_storage_pickup_including_handling') or {}
                section.append(f'<p>Order buffer, whole recording: mean {num(b.get("mean_occupied_slots"),1)} and peak {num(b.get("peak_occupied_slots"))} occupied slots out of {num(b.get("capacity_slots"))}; {num(b.get("still_stored"))} still stored at the boundary. Completed residence mean {num(residence.get("mean_seconds"),1)}s; eligibility-to-pickup mean {num(release.get("mean_seconds"),1)}s including handling. Deposit/retrieval timelines are independently reconciled against slot and aisle occupancy.</p>')
            stations=common['stations'];trs=[]
            for station,d in stations.items():
                def duration(key):return num(d[key]['mean_s'],1) if d.get(key) else '—'
                trs.append(f'<tr><td>EasyLoop {int(station)+1}</td><td class="n">{num(d["mean_assigned"],1)}</td><td class="n">{num(d["mean_residents"],1)}</td><td class="n">{num(d["mean_inventory_conwip"],1)}</td><td class="n">{num(d.get("mean_loaded_awaiting_boarding"),1)}</td><td class="n">{num(d.get("mean_conveyor_inventory"),1)}</td><td class="n">{duration("loaded_to_boarded")}</td><td class="n">{duration("final_pick_to_exit")}</td></tr>')
            section.append('<details><summary>Station preparation, physical occupancy and delays</summary><p>Logical assignments include upstream preparation. Physical resident capacity is 12 order HTMs per EasyLoop (six per side). Inventory pipeline credits include visits awaiting permission to start retrieval; they do not count loaded HTMs. Loaded HTMs awaiting boarding and inventory totes on the conveyor up to pick completion are shown separately. Counts are time averages including unfinished visits; completed-visit delays can understate outstanding long waits.</p><div class="scroll"><table><thead><tr><th>Station</th><th>Assigned orders</th><th>Resident order HTMs</th><th>Inventory pipeline credits</th><th>Loaded inventory HTMs</th><th>Conveyor inventory before pick</th><th>Loaded → conveyor (s)</th><th>Final pick → exit (s)</th></tr></thead><tbody>'+''.join(trs)+'</tbody></table></div></details>')
            if c['traffic_path'].exists():
                t=read(c['traffic_path']);eligible=[r for r in t['cells'] if r['exclusion'] is None]
                top=sorted(eligible,key=lambda r:r['wait_s'],reverse=True)[:12]
                moving=[r for r in eligible if r['moving_s']>0]
                distance=sum(r['distance_m'] for r in moving);seconds=sum(r['moving_s'] for r in moving)
                bins=[sum(r['speed_seconds'][i] for r in moving) for i in range(11)]
                median=next((i for i in range(11) if sum(bins[:i+1])>=sum(bins)/2),None) if sum(bins)>0 else None
                share=100*sum(bins[:2])/sum(bins) if sum(bins)>0 else 0
                c['traffic_summary']=dict(wait_h=t['eligible_wait_s']/3600,moving_h=seconds/3600,
                    moving_m_s=distance/seconds if seconds else None,slow_pct=share,
                    fast_pct=100*bins[8]/sum(bins) if sum(bins)>0 else 0,
                    hours=t['until_ms']/3_600_000,top=top[0] if top else None)
                section.append(f'<details><summary>Cell waiting, movement and congestion evidence</summary><p>First {t["until_ms"]/3_600_000:.2f}h: {t["eligible_wait_s"]/3600:.2f} HTM-hours of waiting in eligible travel cells. Moving mean in those cells: {distance/seconds if seconds else 0:.2f} m/s; {share:.1f}% of moving time below 1m/s. Median-containing speed bin: {f"{median*.5:.1f}–{(median+1)*.5:.1f}m/s" if median is not None and median<10 else "≥5m/s" if median is not None else "unobserved"}. Stops and rotation are separate from moving speed.</p>')
                section.append(traffic_figure(c,t))
                section.append(speed_figure(c,bins))
                trs=[]
                for row in top:
                    frac=100*row['wait_s']/(t['until_ms']/1000)
                    label=row['name'] or row['entity'] or str(row['cell'])
                    trs.append(f'<tr><td title="{esc(label)}">c{row["cell"]}<br><small>{esc(row["kind"])} / zone {row["zone"]}</small></td><td class="n">{num(row["x_mm"]/1000,1)}, {num(row["y_mm"]/1000,1)}</td><td class="n">{num(row["wait_s"],1)}</td><td class="n">{frac:.2f}%</td><td class="n">{num(row["mean_m_s"],2)}</td><td class="n">{num(row["moving_s"],1)}</td></tr>')
                section.append('<div class="scroll"><table><thead><tr><th>Cell</th><th>Position (m)</th><th>Wait (HTM-s)</th><th>Wait / window</th><th>Moving m/s</th><th>Moving HTM-s</th></tr></thead><tbody>'+''.join(trs)+'</tbody></table></div><p>“Wait / window” is aggregate stationary HTM-time divided by elapsed time, not a probability of an arriving robot being delayed. Implicit idle holds and explicit waits are retained in the JSON; reasons are not fully classified.</p>')
                section.append(f'<p>Canonical/playback motion difference: {t["motion_difference_ms"]:.6f}ms; missing-motion slices: {t["missing_motion_slices"]}. Distance and duration use the production integrator, split equally by distance between link endpoints.</p></details>')
            section.append(f'<p class="source">Evidence: <a href="{esc(folder/"report-facts.json")}">report facts</a> · <a href="{esc(folder/"flow-audit.json")}">flow audit</a> · <a href="{esc(folder/"buffer-audit.json")}">buffer audit</a> · <a href="{esc(folder/"traffic.json")}">cell traffic</a> · <a href="{esc(folder/"backlog.json")}">order backlog</a> · <a href="{esc(folder/"state.json")}">run metadata</a></p>')
        elif c['state'].get('status')=='failed':
            if c['stock_preflight']:
                key='empty' if c['preload_fraction']==0 else 'partial';d=source['preload'][key]
                section.append(f'<p class="note">The initial stock cannot fulfill the full order file: {num(d["missing_units"])} units short across {num(d["missing_skus"])} SKUs. Putaway is disabled. This is an input-feasibility failure, not zero-throughput congestion evidence.</p>')
            else:section.append('<p class="note">The simulation failed. No successful throughput result is claimed. See the preserved run log for diagnosis.</p>')
            section.append(f'<a href="{esc(folder/"run.log")}">Run log</a>')
        elif c['analysis_error']:section.append(f'<p class="note">Analysis failed: {esc(c["analysis_error"])}</p>')
        else:section.append(f'<p class="pending">{esc(c["state"].get("status","pending"))}. {esc(c["progress"])}</p>')
        section.append(f'<details><summary>Exact scenario configuration</summary><pre>{esc((folder/"spec.json").read_text())}</pre></details></section>')
        details.append(''.join(section))

    findings=[]
    for id,title in [('full-off-t0','Full preload, T0 orders'),('partial-on-t0-order_release','Partial preload with putaway'),('empty-on-t0-order_release','Empty start, order-priority induction')]:
        c=by_id[id]
        if c['facts']:
            w=c['facts']['windows']['common'];findings.append(f'<li><strong>{title}:</strong> {w["picks_per_hour"]:.1f} picked units/h over {w["hours"]:.2f}h; peak full hour {w["peak_full_hour_picks"]:,}.</li>')
    full=by_id['full-off-t0'];partial=by_id['partial-on-t0-order_release']
    if full['facts'] and partial['facts']:
        fw=full['facts']['windows']['common'];pw=partial['facts']['windows']['common']
        if fw['hours']==pw['hours']==16:
            inducted=sum(pw['inducted_units_by_hour'].values());stores=sum(pw['stored_totes_by_hour'].values())
            findings.append(f'<li><strong>Partial preload sustained picking while replenishing:</strong> {inducted:,} units inducted ({inducted/16:.1f}/h) and {stores:,} filled totes stored ({stores/16:.1f}/h), with pick throughput {100*(pw["picks_per_hour"]/fw["picks_per_hour"]-1):+.2f}% relative to full preload. HTM busy time rose from {100-fw["duty_htm_idle_pct"]:.1f}% to {100-pw["duty_htm_idle_pct"]:.1f}%; VTM busy time from {100-fw["duty_vtm_idle_pct"]:.1f}% to {100-pw["duty_vtm_idle_pct"]:.1f}%. This single-seed comparison also changes stock quantity and placement, so the small pick-rate difference is not an isolated causal effect of putaway.</li>')
            findings.append(f'<li><strong>Operator headroom remains:</strong> the full and partial runs completed {fw["transfer_events"]/16:.1f} and {pw["transfer_events"]/16:.1f} transfer tasks/h, respectively—{100*fw["transfer_events"]/16/1440:.1f}% and {100*pw["transfer_events"]/16/1440:.1f}% of the zero-loss service bound. Multi-unit transfers explain why units/hour looks closer to 1,440 than task service does.</li>')
    empty=by_id['empty-on-t0-order_release'];legacy=by_id['empty-on-t0-sku_demand']
    if empty['facts'] and legacy['facts']:
        ew=empty['facts']['windows']['common'];lw=legacy['facts']['windows']['common']
        findings.append(f'<li><strong>Inbound sequence matters more than total inbound units in the empty-start tests:</strong> order-priority sequencing produced {ew["picks"]:,} picks, compared with {lw["picks"]:,} under demand-ranked SKU sequencing over the same {ew["hours"]:g}h. The two runs inducted {sum(ew["inducted_units_by_hour"].values()):,} and {sum(lw["inducted_units_by_hour"].values()):,} units, respectively. Supplying many units does not help an assigned order whose required SKU is still absent.</li>')
    source_full=by_id['full-off-source']
    if source_full['facts']:
        sw=source_full['facts']['windows']['whole'];sf=source_full['flow']['flow_journal_counts']
        findings.append(f'<li><strong>Full preload with source timing:</strong> {sw["picks"]:,} units picked in {sw["hours"]:.2f}h; peak full-hour throughput {sw["peak_full_hour_picks"]:,} at T={sw["peak_hour_start"]}–{sw["peak_hour_start"]+1}h. The extraction journal records {sf.get("order_extract_presentations",0):,} completed order-container extractions and {sf.get("order_extracted_units",0):,} extracted units. The overall {sw["picks_per_hour"]:.1f}/h average includes the sparse start and late arrivals. The run stopped before its 52h limit after finishing the workload.</li>')
    timed=by_id['empty-on-source-order_release']
    if timed['backlog'] and timed['facts']:
        snapshot=timed['backlog']['snapshots'][-1];w=timed['facts']['windows']['whole']
        findings.append(f'<li><strong>Timed empty start exposes blocking in order assignment:</strong> {w["picks"]:,} units picked in {w["hours"]:.0f}h, with a peak full hour of {w["peak_full_hour_picks"]:,}. At the endpoint, {snapshot["waiting_for_never_inducted_sku"]:,} assigned orders needed a SKU never yet inducted, while {snapshot["individually_stock_ready_unassigned"]:,} released, unassigned orders were individually stock-ready. Median assignment wait was {snapshot["median_assignment_wait_hours"]:.1f}h. This directly supports an assignment/induction mismatch; it is not evidence that traffic congestion caused the collapse. The stock-ready candidates can compete for the same units.</li>')
        if timed['flow'].get('errors') and 'cutoff_bucket' not in timed['flow']:
            findings.append('<li><strong>A cutoff discrepancy remains visible:</strong> the timed empty run\'s summary and journal exclude future fills. The audit also sums a zero-duration terminal bucket that includes a fill beyond the endpoint. Physical pick counts, buffer occupancy and canonical motion reconcile; the flow audit stays flagged and the inventory snapshot is identified separately.</li>')
    lead=by_id['empty-on-source-order_release-lead1h']
    if lead['facts'] and timed['facts']:
        lead_window=lead['facts']['windows']['common'];control=timed['facts']['windows']['common']
        findings.append(f'<li><strong>A synthetic one-hour inbound lead did not resolve timed empty-start starvation:</strong> {lead_window["picks"]:,} units picked in the first 16h ({lead_window["picks_per_hour"]:.1f}/h), versus {control["picks"]:,} ({control["picks_per_hour"]:.1f}/h) when all order-priority lots were eligible from T0. Its peak was higher ({lead_window["peak_full_hour_picks"]:,}/h), but the sustained outcome was worse. Peak throughput alone misses the later starvation.</li>')
    finding_html='<ul>'+''.join(findings)+'</ul>' if findings else '<p>Comparable current-build results will appear here after their recordings and audits finish.</p>'
    policy_html=''
    policy_cases=[by_id[id] for id in ('partial-on-source-order_release','partial-on-source-sku_demand','partial-on-source-order_release-lead1h')]
    if all(c['facts'] and c['facts']['windows']['common']['hours']==16 for c in policy_cases):
        control,ranked,lead_case=policy_cases
        cw,rw,lw=[c['facts']['windows']['common'] for c in policy_cases]
        ci,ri,li=[sum(w['inducted_units_by_hour'].values()) for w in (cw,rw,lw)]
        policy_html=f'<h3>Partial preload masks some induction-policy problems</h3><p>With source order timing, order-priority, demand-ranked and one-hour-lead induction averaged {cw["picks_per_hour"]:.1f}, {rw["picks_per_hour"]:.1f} and {lw["picks_per_hour"]:.1f} picks/hour over 16h. The differences are small in this window, because preload already covers most demand.</p><p><strong>The one-hour-lead case carries very little inbound work:</strong> it inducted only {li:,} units, compared with {ci:,} under T0-eligible order-priority induction and {ri:,} under demand-ranked induction. Only the remaining order shortages receive priority lots. The FIFO sequence leaves the bulk surplus behind later priority lots whose release time has not arrived. This case therefore cannot establish simultaneous high-rate picking and putaway capacity.</p>'
        if ranked['backlog']:
            snap=ranked['backlog']['snapshots'][-1]
            policy_html+=f'<p>The demand-ranked case nevertheless ended with {snap["assigned_waiting_for_stock"]} assigned orders still unprepared; {snap["waiting_with_missing_stock"]} lacked sufficient unreserved stock. Their median assignment wait was {num(snap["median_assignment_wait_hours"],2)}h, while {snap["individually_stock_ready_unassigned"]} released, unassigned orders were individually stock-ready. Near-baseline aggregate throughput can coexist with starvation of particular orders. A longer full-file partial-preload run remains necessary after fixing this policy.</p>'
    t0_ranked=by_id['partial-on-t0-sku_demand']
    if t0_ranked['facts'] and partial['facts']:
        rw=t0_ranked['facts']['windows']['common'];cw=partial['facts']['windows']['common']
        if rw['hours']==cw['hours']==16:
            policy_html+=f'<p>With partial preload and all orders at T0, demand-ranked induction produced {rw["picks"]:,} picks ({rw["picks_per_hour"]:.1f}/h), compared with {cw["picks"]:,} ({cw["picks_per_hour"]:.1f}/h) under order-priority induction. Inducted volumes were {sum(rw["inducted_units_by_hour"].values()):,} and {sum(cw["inducted_units_by_hour"].values()):,} units respectively. These measurements use the same preload and sixteen-hour horizon.</p>'
            if t0_ranked['backlog']:
                snapshot=t0_ranked['backlog']['snapshots'][-1]
                policy_html+=f'<p>The T0 demand-ranked case ended with {snapshot["assigned_waiting_for_stock"]} assigned, unprepared orders, of which {snapshot["waiting_with_missing_stock"]} lacked sufficient unreserved stock. Its headline rate therefore needs to be read alongside the remaining order backlog.</p>'
    congestion_rows=[]
    for id,label in [('full-off-t0','Full preload · no putaway'),('partial-on-t0-order_release','Partial · order-priority putaway'),('empty-on-t0-order_release','Empty · order-priority putaway'),('partial-on-source-order_release','Partial · source-timed orders')]:
        t=by_id[id].get('traffic_summary')
        if t:
            congestion_rows.append(f'<tr><td><a href="#{id}">{label}</a></td><td class="n">{t["hours"]:.0f}</td><td class="n">{t["wait_h"]:.2f}</td><td class="n">{100*t["wait_h"]/(230*t["hours"]):.2f}%</td><td class="n">{t["moving_h"]:.1f}</td><td class="n">{t["moving_m_s"]:.2f}</td><td class="n">{t["slow_pct"]:.1f}%</td></tr>')
    congestion_html='<div class="scroll"><table><thead><tr><th>Run</th><th>Hours</th><th>Travel-cell waiting (HTM-h)</th><th>Share of total fleet-time</th><th>Moving exposure (HTM-h)</th><th>Moving mean (m/s)</th><th>Moving time below 1m/s</th></tr></thead><tbody>'+''.join(congestion_rows)+'</tbody></table></div>' if congestion_rows else ''
    if full.get('traffic_summary') and partial.get('traffic_summary'):
        ft=full['traffic_summary'];pt=partial['traffic_summary'];hot=pt['top']
        congestion_html+=f'<p>The two saturated-order tests accumulated {ft["wait_h"]:.2f} and {pt["wait_h"]:.2f} HTM-hours of ordinary travel-cell waiting. Adding putaway increased movement exposure, while mean moving speed stayed close: {ft["moving_m_s"]:.2f} versus {pt["moving_m_s"]:.2f}m/s. The fleet-wide averages therefore show no broad collapse in this comparison.</p>'
        if hot:
            congestion_html+=f'<p><strong>The station apron still warrants attention.</strong> In the partial-preload run, the largest waiting total was cell c{hot["cell"]}, at ({hot["x_mm"]/1000:.1f}, {hot["y_mm"]/1000:.1f})m: {hot["wait_s"]/3600:.2f} HTM-hours, or {100*hot["wait_s"]/(pt["hours"]*3600):.1f}% of the observation window. Adjacent apron cells recur among the largest hotspots in both high-load runs. These waits are explicit recorded waits; reasons can include station admission and reserved traffic. Their totals alone cannot separate the two causes.</p>'
        congestion_html+=f'<p>The high-load speed distribution has substantial slow and fast exposure: {pt["slow_pct"]:.1f}% of moving time below 1m/s and {pt["fast_pct"]:.1f}% between 4 and 4.5m/s. This is consistent with a mixture of acceleration, crossing/turning approaches and straight travel; it does not establish that slow waves interrupt fast waves. The next traffic diagnosis should inspect direction and time around the apron hotspots.</p>'
    queueing_example=''
    if partial['facts']:
        window=partial['facts']['windows']['common'];station=window['stations']['0']
        loaded=station.get('mean_loaded_awaiting_boarding');delay=station.get('loaded_to_boarded')
        if loaded is not None and delay:
            boarding_rate=delay['n']/window['hours']
            queueing_example=f'<p>For example, EasyLoop 1 in the partial/T0 run averaged {loaded:.2f} loaded inventory HTMs awaiting boarding, with {boarding_rate:.1f} boardings/hour. Dividing that WIP by the completion rate gives about {3600*loaded/boarding_rate:.1f}s in this stage, close to the directly observed completed-visit mean of {delay["mean_s"]:.1f}s. The small difference includes startup and unfinished visits at the boundary. This is the stage-specific comparison relevant to robot occupancy; the roughly 42 logical credits include earlier preparation waiting.</p>'
    css='''
:root{--paper:#f5f5f5;--ink:#2d3142;--muted:#4f5d75;--rule:#d4d6db;--accent:#eb6c36;--link:#2e5aa8}
*{box-sizing:border-box}body{margin:0;background:var(--paper);color:var(--ink);font:16px/1.6 Geist,Arial,sans-serif}main{max-width:1360px;margin:auto;padding:48px 40px 80px}h1{font:52px/1.05 'Instrument Serif',Georgia,serif;margin:18px 0 24px;max-width:850px}h2{font:34px/1.15 'Instrument Serif',Georgia,serif;margin-top:64px}h3{font-size:22px;line-height:1.3}p{max-width:1000px}a{color:var(--link);text-underline-offset:3px}.eyebrow,.source{font:12px/1.6 'Geist Mono',Menlo,monospace;color:var(--muted)}.eyebrow{text-transform:uppercase;letter-spacing:.15em}.lede{font-size:21px;max-width:1020px}.note{border-left:4px solid var(--accent);padding:12px 20px;background:#eb6c360d}.pending{color:var(--muted);font-style:italic}nav{display:flex;flex-wrap:wrap;gap:20px;padding:18px 0;border-block:1px solid var(--rule)}table{border-collapse:collapse;width:100%;font-size:14px}th,td{padding:11px 12px;text-align:left;border-bottom:1px solid var(--rule);vertical-align:top}th{font-size:12px;color:var(--muted);font-weight:600}thead{background:#eceef0}.n{text-align:right;font-variant-numeric:tabular-nums}small{font-size:11px;color:var(--muted)}.scroll{overflow-x:auto}figure{margin:24px 0 38px}svg{width:100%;height:auto}svg text{font:14px Geist,Arial,sans-serif;fill:var(--muted)}figcaption{font-size:13px;color:var(--muted);max-width:1000px}details{border-top:1px solid var(--rule);padding:14px 0}summary{cursor:pointer;font-weight:600}pre{white-space:pre-wrap;overflow-wrap:anywhere;font:12px/1.5 Menlo,monospace;max-height:540px;overflow:auto;background:#eceef0;padding:18px}.case{border-top:2px solid var(--rule);margin-top:42px;padding-top:16px}.cols{display:grid;grid-template-columns:1.25fr 1fr;gap:40px}.audit{font-weight:600}.metric{font-size:28px}.footer{border-top:1px solid var(--rule);margin-top:64px;padding-top:18px;color:var(--muted)}@media(max-width:760px){main{padding:24px 18px}h1{font-size:38px}.cols{grid-template-columns:1fr}th,td{padding:8px;font-size:12px}}@media print{body{background:white}main{max-width:none;padding:0}nav{display:none}details> :not(summary){display:block}pre{max-height:none}figure,table{break-inside:avoid}h2,h3{break-after:avoid}}
'''
    page=f'''<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Walmart · pick, putaway and congestion report</title><style>{css}</style></head><body><main>
<p class="eyebrow">DFW6 simulation · September 14, 2026 · {esc(status)}</p><h1>Picking, putaway<br>and the traffic between them</h1>
<p class="lede">A controlled comparison of inventory starting state, order release timing and synthetic induction policy, with physical completion counts and cell-level traffic evidence.</p>
<p class="source">Generated {esc(now())}. {len(audited)} recordings analyzed; {len(waiting)} queued/running; {len(awaiting_audit)} awaiting audit; {len(rejected)} expected stock-preflight rejections; {len(failures)} run/analysis failures or audit discrepancies. Source {manifest['source_commit'][:9]}. Seed 0. All orders retained.</p>
<nav><a href="#findings">Findings</a><a href="#matrix">Matrix</a><a href="#sources">Source files</a><a href="#congestion">Congestion</a><a href="#controls">Controls &amp; limitations</a><a href="#details">Run evidence</a></nav>
<section id="findings"><h2>What the evidence supports</h2>{finding_html}
<p class="note">These are simulation experiments, not a certified warehouse capacity or congestion assessment. The physical VTM model, decant behavior and outbound simplifications still differ from the proposed installation. A valid run can measure the implemented model while leaving those site questions open.</p>
<p>The earlier eight-hour partial-preload experiment completed <strong>9,651 picks (1,206.4/h)</strong>, 10,719 inducted units (1,339.9/h), 2,058 filled totes and 2,052 completed rack stores. Its full-preload/no-putaway baseline completed 9,549 picks (1,193.6/h). Peak full-hour picks were 1,242 and 1,246 respectively. This is encouraging shared-fleet behavior; it is not a clean estimate of putaway’s isolated effect because preload quantity and stock placement also changed.</p>
<p>The earlier empty-start run completed only 28 picks in eight hours. Its synthetic induction ranked SKUs by total file demand, while orders were assigned by ID. This mismatch delayed required SKUs by thousands of induction lots. The new comparison retains that policy as a control and tests order-priority sequencing explicitly.</p><p><strong>Cold-start supply has a separate capacity limit.</strong> With one SKU per synthetic inbound tote and at most six units per fill, the file requires at least 21,182 demand-serving fills. Six stations at 60 fills/hour need at least <strong>58.8 hours</strong> of ideal induction to supply all requested units from empty, before transport losses. Inducting the entire on-hand snapshot needs at least 329,814 fills (916.2 hours at that ideal rate). These bounds apply to the current single-SKU-per-fill synthetic model. General inventory supports mixed-SKU totes, but synthetic decant does not yet generate mixed-SKU fills. Raising the unit cap alone barely changes the distinct-SKU requirement. These are cumulative service lower bounds, not predictions of a peak hourly pick rate. Partial preload leaves only 378 demanded units missing, so it tests a fundamentally different starting condition.</p></section>
<section id="matrix"><h2>The experiment matrix</h2><p>Rates below use the first <strong>16 simulated hours</strong> of each completed recording. A dash means no result, not zero. Two source-timed cases have a limit of 52h and are reported separately; a finished workload can stop earlier. Preload fractions are 100%, 50% and 0% of each SKU, rounded upward; partial is therefore 747,243 units (53.46%), in half the eligible stocked-tote positions.</p>
<div class="scroll"><table><thead><tr><th>Preload</th><th>Order release</th><th>Induction policy</th><th>Evidence status</th><th>Picks/h</th><th>Peak 1h picks</th><th>Inducted units/h</th><th>Rack stores/h</th></tr></thead><tbody>{''.join(rows)}</tbody></table></div>
<p>“Order-priority” orders the inbound lots by source release then order ID, or order ID alone for T0 experiments. Preloaded units and surplus from earlier lots cover demand before another priority lot is generated. All lots are available for induction at T0 unless the explicit one-hour lead sensitivity is selected. Availability does not teleport inventory: empty-tote retrieval, decant, HTM travel and VTM rack storage still take physical time.</p>
<p><strong>Static induction and live order assignment are not identical policies.</strong> The order dispatcher chooses the earliest eligible assignment time and then order ID. Once several orders are already released, their IDs determine priority; the induction queue remains sorted by original release time. The source-timed empty-start case can therefore develop another supply/assignment mismatch as backlog grows. The T0 case shares ID priority. These experiments test a fixed induction sequence, not dynamic reprioritization for the currently assigned orders. Any resulting throughput loss must be distinguished from congestion.</p>
<details><summary>Queue sizing and what the WIP measurements mean</summary><p>The implemented pipeline ceiling is <code>ceil(queue_factor × F_max / service_period) + 2</code>. With queue factor 3, a roughly 133-second estimated fetch path and a 10-second operator period, the station limits are 42–43 inventory visits. The two extra credits are the existing presentation/estimation margin. This is a sizing heuristic; it does not guarantee saturation when shared robots, stock or traffic add delays.</p><p>Little’s law relates observed mean WIP, throughput and mean residence time for the same population and stage. A logical inventory credit, a loaded HTM approaching the conveyor, a tote on the conveyor, and an order HTM inside its waiting area are different populations. Each has a separate count and lifetime here. Physical order-area admission remains limited to six residents per side, regardless of how much upstream work is prepared.</p>{queueing_example}</details>
{charts}{policy_html}{extended}</section>
<section id="sources"><h2>What the supplied files actually say</h2><div class="cols"><div><h3>Stock is a starting snapshot</h3><p><code>onhand.csv</code> contains <strong>{source.get('stock_skus',0):,} SKUs and {source.get('stock_units',0):,} units</strong>. Fields include item dimensions, velocity band and <code>on_hand_qty</code>. It contains no induction, receiving, ASN, truck-arrival or tote-location timestamps. The approved demo overlay adds 43 units across 40 SKUs, yielding 1,397,632 units.</p><p>The snapshot does not specify how stock is divided among physical totes. Automatic preload distributes it across eligible rack totes; mixed SKUs and multiple units are allowed. Empty start means empty source totes already exist in the racks, not an empty building with no containers.</p></div><div><h3>Orders have release times</h3><p><code>outbound.csv</code> contains <strong>{source.get('orders',0):,} orders, {source.get('order_lines',0):,} lines and {source.get('ordered_units',0):,} requested units</strong>, covering {source.get('demand_skus',0):,} SKUs. Drops span {source.get('first_drop','')} through {source.get('last_drop','')}—{source.get('drop_span_hours',0):.2f} hours. No timezone offset is present.</p><p>Only {first6:,} orders arrive in the first six hours. Every order’s lines agree on its drop timestamp. <code>cutoff_time</code> contains time-of-day values without a date; its precise operational meaning and relationship to truck release are unconfirmed. Buffer hold in these experiments is a synthetic one hour after storage.</p></div></div>
<p>Automatic preload uses {num(source.get('preload',{}).get('full',{}).get('stocked_totes'))} stocked inventory totes for the full snapshot, averaging {num(source.get('preload',{}).get('full',{}).get('mean_units_per_stocked_tote'),2)} units each. Partial preload uses {num(source.get('preload',{}).get('partial',{}).get('stocked_totes'))} stocked totes, averaging {num(source.get('preload',{}).get('partial',{}).get('mean_units_per_stocked_tote'),2)} units each. The map exposes {num(source.get('preload',{}).get('full',{}).get('source_storage_slots'))} source storage positions. The configured 100 units/tote is a packing limit; it does not mean every preloaded tote contains 100 units. The loader spreads the finite stock across its selected totes.</p>
<p><strong>Sending the two files together establishes neither an inbound schedule nor a cold-start commissioning sequence.</strong> We synthesize inbound work by subtracting preload from the on-hand snapshot. Full preload leaves no inbound work; partial leaves 650,389 units; empty leaves all 1,397,632 units to induct. With putaway disabled, partial stock is 378 units short across 321 required SKUs, and empty stock cannot satisfy any order.</p>
<p>Order mix: {source.get('order_traits',{}).get('single_unit',0):,} single-unit orders and {source.get('order_traits',{}).get('multiple_units',0):,} multiple-unit orders; {source.get('order_traits',{}).get('multiple_lines',0):,} multiline orders. Containers: {esc(source.get('containers',{}))}. Destinations: {esc(source.get('destinations',{}))}. This mixture matters: early T0 tests disproportionately exercise single-unit orders because internal IDs follow source-ID string order.</p>
{source_chart}<details><summary>Exact source demand by elapsed hour</summary><table><thead><tr><th>Interval</th><th>Released orders</th><th>Released units</th></tr></thead><tbody>{hourly_source}</tbody></table></details>
<details><summary>File columns and input feasibility</summary><pre>{esc(json.dumps(source,indent=2))}</pre></details></section>
<section id="congestion"><h2>Observed traffic and congestion questions</h2>{congestion_html}<p>A high-volume map can have a local choke point while much of the fleet is idle. Conversely, low throughput can come from missing stock or unreleased orders while travel cells are clear. We therefore examine local waiting exposure, moving-speed distributions, inventory residence before conveyor boarding, order residence and final-pick-to-exit delay together.</p>
<ul><li><strong>Stationary waiting:</strong> measured in HTM-seconds per cell. Queue, presentation/access, exchange and charging cells are excluded from the travel-cell color scale. Eligible waiting still includes waiting for work or resources; causal labels are incomplete.</li><li><strong>Moving speed:</strong> distance divided by moving time, with duration-weighted 0.5m/s bins. Stops and turns in place are excluded and measured separately. The median is reported as a containing bin, not an exact value.</li><li><strong>Spatial attribution:</strong> each link contributes half its distance to each endpoint cell. Canonical kinematics, including curves, determine the time split. This avoids treating rendering samples as measurements.</li><li><strong>Window and exposure:</strong> hotspot comparisons use the common first-16h window. The report shows time exposure beside speed; a cell observed for two seconds cannot carry the same confidence as an aisle observed for hours.</li><li><strong>What a cumulative map cannot prove:</strong> two speed populations do not establish interference. Distinct fast and slow waves can be healthy. Directional, time-resolved crossing analysis and reliable wait-cause attribution remain necessary to diagnose that mechanism.</li></ul>
<p>Each completed run below contains its own waiting map and hotspot table. Exact raw cell IDs, entity identities, coordinates, moving-time histograms and implicit idle holds are retained in its traffic JSON. Canonical movement is checked against playback duration. Low observed waiting in this model cannot clear the proposed layout of floor-driving-VTM or real robot-clearance risks. The report is rendered and checked in a headless browser. Production metric, serialization, tooltip-unit and shader checks have passed; a final visual acceptance of the desvis GPU heatmap itself remains outstanding.</p><p>The audit found that some declared waiting berths have an ordinary Open traversal type. The UI source and report exporter now exclude them by equipment membership. The frozen simulation executable predates that UI fix; its simulation physics and recorded movements were preserved for comparison. Rebuild the headed application before the remaining visual heatmap check.</p></section>
<section id="controls"><h2>Fixed controls, capacity bounds and open questions</h2><table><thead><tr><th>Control</th><th>Value and interpretation</th></tr></thead><tbody>
<tr><td>Robots</td><td>230 HTMs; 40 inventory VTMs plus 8 buffer VTMs. The extra buffer fleet remains a site clarification. The DoO lists a different total VTM count; do not silently reconcile incompatible document revisions.</td></tr>
<tr><td>Picking</td><td>Four EasyLoops, one exclusive operator each, two presentation sides, six waiting positions per side. Ten seconds per implemented item-transfer task; 1,440 transfers/hour is the zero-loss four-operator bound. Multiple units can move in one task, so 1,440 is not a universal units/hour ceiling. Unit-dependent handling time remains to be calibrated.</td></tr>
<tr><td>Inbound / decant</td><td>Six stations; provisional 60 tote fills/hour/station; at most six units in a same-SKU inbound lot. Ideal bound: 360 filled totes/hour and at most 2,160 units/hour before transport losses or partial lots. A different DoO figure is 250 units/hour/station. Multiple active decant positions, top-off workflow and inbound-case delivery are not fully represented.</td></tr>
<tr><td>Inventory</td><td>Finite on-hand snapshot, no fabricated replacement supply. Initial plus future stock passes the order sufficiency check. A 1.4-million-unit empty start is a very large replenishment problem; eight or sixteen hours cannot recreate the original stock snapshot at these rates.</td></tr>
<tr><td>Queueing / admission</td><td>Queue factor 3. Logical assignments can exceed the 12 physical order residents per EasyLoop. Physical admission is time-reserved. Inventory release has its own CONWIP bound and consumer timing. WIP must be measured in consistent units before applying a rate × residence-time relationship.</td></tr>
<tr><td>Order containers / outbound</td><td>128 reusable SIOC trays for diagnostic continuity (not a validated site count); ordinary order totes form a separate seeded pool. This does not cap all reusable containers at 128. Ordinary box induction and extraction enabled. Boxes disappear at sealer handoff; sealed-box reinduction is deferred. DS uses the buffer with a one-hour hold; SC direct extraction remains temporary. Tote/tray contents leave at extraction and the container returns to storage.</td></tr>
<tr><td>Motion and traffic</td><td>Kinematic HTM motion and collision avoidance always enabled. Relay entry/back-out and rotation restrictions are modeled. Walmart floor-driving VTMs are not implemented; charging and battery constraints are not modeled. Reservation-based navigation does not establish real-world safety or physical throughput.</td></tr>
<tr><td>Reproducibility</td><td>One frozen release binary and copied inputs, seed 0, no order cap. Its hash and simulation source commit are frozen in the manifest; profile Git metadata is collected at runtime and may reflect later report-only commits. At most four headless runs, RAM-gated. No headed windows launched. One seed is a controlled comparison, not a confidence interval.</td></tr></tbody></table>
<h3>Questions that materially change the result</h3><ol><li>What is the actual inbound receiving/decant schedule, and is the on-hand snapshot already present at the first order drop?</li><li>Should orders lacking allocatable stock remain unassigned, or should decant prioritize inventory for already-assigned orders? Should released orders retain earliest-release priority instead of switching to ID priority when a backlog exists?</li><li>What are the validated decant presentation count, units per tote, active positions, top-off policy and service-time distribution?</li><li>What is the operator time for multiple units versus one unit, including confirmation and switching sides?</li><li>What event releases buffered orders, and what do cutoff_time, destination_type and truck timing mean operationally?</li><li>What are the floor-driving VTM fleet, aisle rules and shared-traffic footprint? What order-tray count and charging workload should be modeled?</li></ol>
<h3>Recommended next experiments</h3><ol><li><strong>Remove inventory starvation from order assignment.</strong> Compare stock-ready admission or induction that prioritizes assigned shortages against the frozen baseline. Preserve physical order-area admission limits and use simulator timestamps for availability. Measure waiting assigned orders and stock-ready unassigned orders alongside picks.</li><li><strong>Calibrate inbound workload.</strong> Obtain actual receiving times and tote contents, then compare the site decant workflow with the current single-SKU synthetic fills. Keep empty-start commissioning separate from steady operation with working inventory.</li><li><strong>Explain operator idle time and apron waiting together.</strong> Attribute each idle interval to order-container readiness, inventory readiness, conveyor/side availability or outbound clearance. Inspect traffic directions and time windows at the identified apron cells before increasing release rates or fleet size.</li><li><strong>Repeat the selected correction.</strong> Use the same stock placement and workload for an isolated comparison, then additional seeds and a longer horizon to check that gains persist. Reintroduce the floor-driving VTM footprint and validated charging/inbound/outbound details before drawing site-capacity conclusions.</li></ol>
<h3>Earlier isolation experiments</h3><p>Before the present outbound layout and buffer corrections, an eight-hour inventory-release run averaged 891.5 picks/h; a one-hour run with extraction bypassed achieved 1,264; an eight-hour run bypassing both extraction and box induction averaged 1,339.25. An earlier VTM-isolation run achieved 635.4/h. These are historical diagnostic results with different code/layout controls, not rows in this matched experiment or evidence that bypasses solve the physical design.</p></section>
<section id="details"><h2>Run evidence and configurations</h2><p>All rates count physical completions through the relevant observation boundary. Inducted units, filled totes and filled-tote rack stores are separate events. Error and preflight cases are retained. Evidence links point to local artifacts; the embedded tables and charts remain readable when this HTML is shared alone.</p>{''.join(details)}</section>
<details><summary>Frozen experiment manifest</summary><pre>{esc(json.dumps(manifest,indent=2))}</pre></details>
<footer class="footer"><p>Source documentation: Walmart Dallas Simulation Design Specs; Simulation Objectives; DFW6 Pilot Design of Operations Draft 2.0; Daniel’s September 12 call notes and subsequent rulings. Document discrepancies remain questions, not silently chosen facts.</p><p class="source">Generated from completed recordings and copied source files by scripts/walmart_matrix/report.py. HTML is self-contained. No estimated result substitutes for a pending recording.</p></footer></main></body></html>'''
    output.parent.mkdir(parents=True,exist_ok=True)
    temporary=output.with_suffix(f'.{os.getpid()}.pending');temporary.write_text(page);temporary.replace(output)
    print(output,len(page),'bytes',len(audited),'analyzed',len(waiting),'waiting')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base',type=Path,default=BASE,help='Matrix artifact directory')
    parser.add_argument('--output',type=Path,default=DEST,help='Destination single-page HTML report')
    args=parser.parse_args()
    main(args.base.resolve(),args.output.resolve())
