#!/usr/bin/env python3
"""Audits and compact report facts, derived from completed profile timestamps.

The watcher never reruns a simulation. It refreshes the HTML as completed
recordings become available, preserving failing preflights and failed audits.
"""
# ARCHIVE TOOL: the DESPROF v4 container this parses was retired with the
# `.worldview` format. It still opens recordings made by binaries from before
# that change; nothing writes the format any more.
import argparse
from collections import Counter, defaultdict
from compression import zstd
import csv
import datetime as dt
import hashlib
import json
import math
from pathlib import Path
import re
import statistics
import struct
import subprocess
import sys
import time
import msgpack

from run import BASE, REPO, now, save
from backlog import diagnose
sys.path.insert(0,str(REPO/'scripts'))
from audit_buffer_profile import FrameReader, skip_bytes

REPORT_OUTPUT=REPO/'docs/easyloop/overnight-2026-09-14/report.html'


def source_audit():
    inputs=BASE/'inputs'
    with (inputs/'onhand.csv').open(newline='') as f:
        reader=csv.DictReader(f);stock_columns=reader.fieldnames;stock_rows=list(reader)
    stock={r['item_id']:int(float(r['on_hand_qty'])) for r in stock_rows}
    with (inputs/'walmart_demo_stock_top_up.csv').open(newline='') as f:
        overlay={r['item_id']:int(r['additional_units']) for r in csv.DictReader(f)}
    with (inputs/'outbound.csv').open(newline='') as f:
        reader=csv.DictReader(f);order_columns=reader.fieldnames;lines=list(reader)
    orders=defaultdict(list)
    for line in lines:orders[line['order_id']].append(line)
    parse=lambda value:dt.datetime.fromisoformat(value)
    earliest=min(parse(line['order_drop_time']) for line in lines)
    latest=max(parse(line['order_drop_time']) for line in lines)
    hours=defaultdict(Counter);demand=Counter();traits=Counter();containers=Counter();destinations=Counter()
    inconsistencies=[]
    for order_id,rows in orders.items():
        dates={r['order_drop_time'] for r in rows}
        if len(dates)!=1:inconsistencies.append(order_id)
        at=(parse(rows[0]['order_drop_time'])-earliest).total_seconds()
        h=int(at//3600)
        qty=sum(int(float(r['order_qty'])) for r in rows)
        hours[h]['orders']+=1;hours[h]['units']+=qty
        traits['single_unit' if qty==1 else 'multiple_units']+=1
        traits['single_line' if len(rows)==1 else 'multiple_lines']+=1
        containers[rows[0]['shipping_protection']]+=1;destinations[rows[0]['destination_type']]+=1
        for row in rows:demand[row['sku_id']]+=int(float(row['order_qty']))
    scenarios={}
    for name,fraction in [('full',1),('partial',.5),('empty',0)]:
        initial={sku:math.ceil((qty+overlay.get(sku,0))*fraction) for sku,qty in stock.items()}
        deficits=[{'sku':sku,'need':qty,'stock':initial.get(sku,0),'deficit':qty-initial.get(sku,0)}
                  for sku,qty in demand.items() if qty>initial.get(sku,0)]
        scenarios[name]=dict(preloaded_units=sum(initial.values()),missing_units=sum(d['deficit'] for d in deficits),
                             missing_skus=len(deficits),deficit_examples=deficits[:10],
                             minimum_demand_lots_at_six=sum(math.ceil(d['deficit']/6) for d in deficits),
                             minimum_stock_lots_at_six=sum(math.ceil((qty+overlay.get(sku,0)-initial[sku])/6) for sku,qty in stock.items()))
    for name,case in [('full','full-off-t0'),('partial','partial-on-t0-order_release'),('empty','empty-on-t0-order_release')]:
        log=BASE/case/'run.log'
        if log.exists():
            with log.open('rb') as f:prefix=f.read(15_000).decode(errors='replace')
            match=re.search(r'\[inventory\] (\d+) SKUs, (\d+) units in (\d+) stocked totes / (\d+) slots',prefix)
            if match:
                skus,units,totes,slots=map(int,match.groups())
                assert units==scenarios[name]['preloaded_units']
                scenarios[name].update(stocked_totes=totes,source_storage_slots=slots,
                                       mean_units_per_stocked_tote=units/totes if totes else None)
    out=dict(stock_columns=stock_columns,order_columns=order_columns,stock_skus=len(stock),stock_units=sum(stock.values()),
             overlay_units=sum(overlay.values()),overlay_skus=len(overlay),orders=len(orders),order_lines=len(lines),
             ordered_units=sum(demand.values()),demand_skus=len(demand),first_drop=earliest.isoformat(),last_drop=latest.isoformat(),
             drop_span_hours=(latest-earliest).total_seconds()/3600,hourly={str(k):v for k,v in sorted(hours.items())},
             order_traits=dict(traits),containers=dict(containers),destinations=dict(destinations),
             inconsistent_order_drop_dates=inconsistencies,preload=scenarios,
             blank_cutoffs=sum(not r['cutoff_time'] for r in lines),cutoff_examples=sorted({r['cutoff_time'] for r in lines})[:8],
             timings_note='Source timestamps have no time-zone offset. T0 is the earliest source order drop. On-hand stock has no induction/receiving timestamps.')
    save(BASE/'source-audit.json',out)
    return out


def describe(values):
    if not values:return None
    values=sorted(values)
    return dict(n=len(values),mean_s=statistics.mean(values)/1000,p50_s=statistics.median(values)/1000,
                p90_s=values[min(len(values)-1,math.ceil(.9*len(values))-1)]/1000,max_s=max(values)/1000)


def window_counts(records,until,key='quantity'):
    hours=Counter()
    for row in records:
        at=row['sim_time_ms']
        if 0<=at<=until:
            bucket=min(at//3_600_000,max(0,math.ceil(until/3_600_000)-1))
            hours[bucket]+=row[key] if key else 1
    return {str(hour):hours[hour] for hour in range(math.ceil(until/3_600_000))}


def scan(profile):
    keep={'sim_time_secs','total_picks','total_putaways','pick_log','putaway_log','completed_order_log','vtm_cycle_log',
          'htm_idle_pct','vtm_idle_pct','htm_fleet_size','vtm_fleet_size','station_timelines','station_utilization',
          'htm_duty_totals','vtm_duty_totals','htm_leg_log'}
    summary={};metadata={};hourly_metrics=defaultdict(Counter);duty_slices=[]
    phases=defaultdict(dict);inventory=defaultdict(dict);ready=defaultdict(dict)
    with profile.open('rb') as raw:
        assert raw.read(8)==b'DESPROF\x04'
        with zstd.open(raw,'rb') as stream:
            while tag:=stream.read(1):
                length=struct.unpack('<Q',stream.read(8))[0]
                if tag!=b'\x7e':skip_bytes(stream,length);continue
                u=msgpack.Unpacker(FrameReader(stream,length),raw=False,strict_map_key=False,max_buffer_size=2**31)
                for _ in range(u.read_map_header()):
                    key=u.unpack()
                    if key=='summary':
                        for _ in range(u.read_map_header()):
                            field=u.unpack()
                            if field in keep:summary[field]=u.unpack()
                            else:u.skip()
                    elif key=='metadata':metadata=u.unpack()
                    elif key=='time_slices':
                        for _ in range(u.read_array_header()):
                            row={}
                            for _ in range(u.read_map_header()):
                                field=u.unpack()
                                if field in ('start_ms','metrics','station_diag'):row[field]=u.unpack()
                                else:u.skip()
                            if row['start_ms']>=round(summary['sim_time_secs']*1000):continue
                            h=row['start_ms']//3_600_000;m=row['metrics']
                            span=min(metadata['slice_width_ms'],round(summary['sim_time_secs']*1000)-row['start_ms'])
                            busy={fleet:sum(m.get(f'{fleet}_duty_{state}_ms',0) for state in
                                    (('traveling','loading_unloading') if fleet=='vtm' else
                                     ('traveling','loading_unloading','processing','waiting_for_operator')))
                                  for fleet in ('htm','vtm')}
                            duty_slices.append((row['start_ms'],span,busy))
                            hourly_metrics[h]['sample_ms']+=metadata['slice_width_ms']
                            for fleet in ('htm','vtm'):
                                hourly_metrics[h][fleet+'_idle_ms']+=m.get(fleet+'_state_Idle',0)*metadata['slice_width_ms']
                            for key in ('pathfinder_queries','pathfinder_failures','backpressure_events','htm_travel_distance_mm',
                                        'htm_duty_wait_loaded_ms','htm_duty_rotating_ms','htm_duty_traveling_ms'):
                                hourly_metrics[h][key]+=m.get(key,0)
                            diag=row.get('station_diag',{})
                            for x in diag.get('order_phases',[]):phases[(x['conveyor_id'],x['order_id'])][x['phase']]=x['at']
                            for x in diag.get('inventory_visit_phases',[]):inventory[(x['conveyor_id'],x['order_id'],x['pick_index'])][x['phase']]=x['at']
                            for x in diag.get('presentation_readiness',[]):ready[(x['conveyor_id'],x['order_id'],x['pick_index'])][x['stream']]=x['ready_at']
                    else:u.skip()
    end=round(summary['sim_time_secs']*1000)
    fills=[r for r in summary['putaway_log'] if r['sim_time_ms']<=end]
    ids={r['presentation_id'] for r in fills if r['presentation_id'] is not None}
    stores=[dict(sim_time_ms=r['sim_time_ms']+r['total_cycle_ms']) for r in summary['vtm_cycle_log']
            if r['direction']=='Return' and r['tote_source']=='Putaway' and r['presentation_id'] in ids]
    windows={}
    for label,until in [('common',min(end,16*3_600_000)),('whole',end)]:
        full_hours=int(until//3_600_000)
        hourly=window_counts(summary['pick_log'],until)
        unit_total=sum(hourly.values())
        rate=unit_total*3_600_000/until if until else 0
        peak=max(((qty,int(hour)) for hour,qty in hourly.items() if int(hour)<full_hours),default=(0,0))
        paired_rows=[r for r in summary['pick_log'] if r['sim_time_ms']<=until]
        sorted_rows=sorted(paired_rows,key=lambda x:x['sim_time_ms'])
        left=0;count=0
        rolling=sum(r['quantity'] for r in sorted_rows if r['sim_time_ms']<3_600_000) if until>=3_600_000 else 0
        rolling_end=3_600_000 if until>=3_600_000 else None
        for right,row in enumerate(sorted_rows):
            count+=row['quantity']
            while row['sim_time_ms']-sorted_rows[left]['sim_time_ms']>=3_600_000:
                count-=sorted_rows[left]['quantity'];left+=1
            if row['sim_time_ms']>=3_600_000 and count>rolling:rolling=count;rolling_end=row['sim_time_ms']
        station={}
        for machine in range(4):
            def occupancy(rows,start,stop):
                return sum(max(0,min(until,v.get(stop,until))-max(0,v[start])) for key,v in rows.items()
                           if key[0]==machine and start in v and v[start]<=until)/max(1,until)
            def interval(rows,start,stop):
                return describe([v[stop]-v[start] for key,v in rows.items()
                                 if key[0]==machine and start in v and stop in v and v[stop]<=until and v[stop]>=v[start]])
            station[machine]=dict(mean_assigned=occupancy(phases,'Assigned','FinalPickCompleted'),
                mean_residents=occupancy(phases,'Admitted','AreaExited'),mean_inventory_conwip=occupancy(inventory,'Released','PickCompleted'),
                mean_loaded_awaiting_boarding=occupancy(inventory,'Loaded','Boarded'),
                mean_conveyor_inventory=occupancy(inventory,'Boarded','PickCompleted'),
                loaded_to_boarded=interval(inventory,'Loaded','Boarded'),boarded_to_pick=interval(inventory,'Boarded','PickCompleted'),
                final_pick_to_exit=interval(phases,'FinalPickCompleted','AreaExited'))
        def duty_idle(fleet):
            busy=sum(values[fleet]*min(span,max(0,until-start))/span for start,span,values in duty_slices)
            return 100*(1-busy/max(1,until*summary[fleet+'_fleet_size']))
        windows[label]=dict(hours=until/3_600_000,full_hours=full_hours,picks=unit_total,picks_per_hour=rate,
            peak_full_hour_picks=peak[0],peak_hour_start=peak[1],rolling_hour_peak=rolling,rolling_peak_end_ms=rolling_end,
            hourly_picks=hourly,hourly_transfers=window_counts(summary['pick_log'],until,None),
            inducted_units_by_hour=window_counts(fills,until),filled_totes_by_hour=window_counts(fills,until,None),
            stored_totes_by_hour=window_counts(stores,until,None),transfer_events=len(paired_rows),
            multiunit_transfer_events=sum(r['quantity']>1 for r in paired_rows),stations=station,
            duty_htm_idle_pct=duty_idle('htm'),duty_vtm_idle_pct=duty_idle('vtm'))
    duty_differences={fleet:windows['whole']['duty_'+fleet+'_idle_pct']-100*summary[fleet+'_idle_pct'] for fleet in ('htm','vtm')}
    return dict(profile=str(profile),recorded_hours=end/3_600_000,windows=windows,metadata=metadata,
                duty_summary_difference_pp=duty_differences,
                summary_idle_pct={fleet:100*summary[fleet+'_idle_pct'] for fleet in ('htm','vtm')})


def audit_case(case):
    folder=BASE/case['id'];profiles=list(folder.glob('*.desprofile'))
    if len(profiles)!=1:raise ValueError(f'Expected one recording in {folder}, found {len(profiles)}')
    profile=profiles[0]
    outcomes={}
    for kind in ('flow','buffer'):
        output=folder/(kind+'-audit.json')
        cmd=[sys.executable,str(REPO/f'scripts/audit_{kind}_profile.py'),str(profile),'--output',str(output)]
        if kind=='buffer':cmd+=['--zones','21,22,23,24,25,26,27,28']
        with (folder/(kind+'-audit.log')).open('w') as log:
            outcomes[kind]=subprocess.run(cmd,cwd=REPO,stdout=log,stderr=subprocess.STDOUT).returncode
    traffic=REPO/'target/release/traffic_report'
    with (folder/'traffic-audit.log').open('w') as log:
        outcomes['traffic']=subprocess.run([str(traffic),str(profile),str(folder/'spec.json'),'16',str(folder/'traffic.json')],
                                          cwd=REPO,stdout=log,stderr=subprocess.STDOUT).returncode
    facts=scan(profile)
    facts['audit_exit_codes']=outcomes
    save(folder/'report-facts.json',facts)
    try:diagnose(folder,base=BASE)
    except Exception as error:save(folder/'backlog-error.json',dict(error=repr(error),at=now()))


def refresh():
    subprocess.run([sys.executable,str(REPO/'scripts/walmart_matrix/report.py'),
                    '--base',str(BASE),'--output',str(REPORT_OUTPUT)],cwd=REPO,check=True)


def refresh_completed():
    """Recheck flow cutoffs and report facts without rerunning simulations."""
    manifest=json.loads((BASE/'manifest.json').read_text())
    for case in manifest['cases']:
        folder=BASE/case['id'];fact_path=folder/'report-facts.json'
        if not fact_path.exists():continue
        previous=json.loads(fact_path.read_text());profile=Path(previous['profile'])
        flow_path=folder/'flow-audit.json';archive=folder/'flow-audit-before-cutoff-fix.json'
        if not archive.exists():archive.write_bytes(flow_path.read_bytes())
        output=folder/'flow-audit.pending.json'
        outcome=subprocess.run([sys.executable,str(REPO/'scripts/audit_flow_profile.py'),str(profile),
                                '--output',str(output)],cwd=REPO).returncode
        facts=scan(profile);facts['audit_exit_codes']=dict(previous['audit_exit_codes'],flow=outcome)
        output.replace(flow_path);save(fact_path,facts)
        print(case['id'],'flow audit',outcome,flush=True)
        refresh()


def watch():
    import fcntl
    lock=(BASE/'analysis.lock').open('a');fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
    source_audit()
    manifest=json.loads((BASE/'manifest.json').read_text())
    while True:
        waiting=False
        for case in manifest['cases']:
            folder=BASE/case['id'];state=json.loads((folder/'state.json').read_text())
            if state['status'] in ('queued','running'):waiting=True;continue
            if (folder/'report-facts.json').exists() or (folder/'analysis-error.json').exists():continue
            if state['status']=='finished':
                try:audit_case(case)
                except Exception as error:save(folder/'analysis-error.json',dict(error=repr(error),at=now()))
            refresh()
        refresh()
        save(BASE/'analysis-status.json',dict(pid=__import__('os').getpid(),updated_at=now(),waiting=waiting))
        if not waiting:break
        time.sleep(30)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('case',nargs='?',help='Case ID to audit, replacing its analysis outputs')
    parser.add_argument('--base',type=Path,default=BASE,help='Matrix artifact directory')
    parser.add_argument('--report-output',type=Path,default=REPORT_OUTPUT,
                        help='HTML output for watcher/refresh operations')
    actions=parser.add_mutually_exclusive_group()
    actions.add_argument('--watch',action='store_true',help='Audit missing finished cases and refresh HTML')
    actions.add_argument('--refresh-completed',action='store_true',help='Recheck existing flow audits and report facts')
    actions.add_argument('--sources',action='store_true',help='Rebuild source-audit.json from the frozen CSVs')
    args=parser.parse_args()
    if bool(args.case)==any((args.watch,args.refresh_completed,args.sources)):
        parser.error('Choose one case ID or one action flag')
    BASE=args.base.resolve()
    REPORT_OUTPUT=args.report_output.resolve()
    if args.watch:watch()
    elif args.refresh_completed:refresh_completed()
    elif args.sources:print(json.dumps(source_audit(),indent=2))
    else:
        manifest=json.loads((BASE/'manifest.json').read_text())
        case=next((c for c in manifest['cases'] if c['id']==args.case),None)
        if case is None:parser.error(f'Unknown case ID: {args.case}')
        audit_case(case)
