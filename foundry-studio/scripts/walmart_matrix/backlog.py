#!/usr/bin/env python3
"""Explain recorded order preparation waits against the frozen stock ledger.

The Walmart loader assigns SKU IDs in on-hand row order and order IDs in source
label order. PreparationStarted is emitted only after all order units have been
reserved. Unreserved supply is therefore preload + completed fills - all units
claimed by orders whose preparation has started. Candidate readiness is checked
individually: it does not assert that all candidates can reserve shared stock.
"""
# ARCHIVE TOOL: the DESPROF v4 container this parses was retired with the
# `.worldview` format. It still opens recordings made by binaries from before
# that change; nothing writes the format any more.
from collections import Counter, defaultdict
from compression import zstd
import csv
import datetime as dt
import json
import math
from pathlib import Path
import statistics
import struct
import sys

import msgpack

from run import BASE, REPO, save
sys.path.insert(0,str(REPO/'scripts'))
from audit_buffer_profile import FrameReader, skip_bytes


def diagnose(folder, base=BASE):
    profile=next(folder.glob('*.desprofile'))
    spec=json.loads((folder/'spec.json').read_text())
    config=next(t.get('config',{}) for t in spec['composition']['tasks'] if t['type']=='FillInventory')
    fraction=config.get('preload_fraction',1)
    with (base/'inputs/onhand.csv').open(newline='') as f:stock=list(csv.DictReader(f))
    sku_ids={r['item_id']:i for i,r in enumerate(stock)}
    with (base/'inputs/walmart_demo_stock_top_up.csv').open(newline='') as f:
        overlay={r['item_id']:int(r['additional_units']) for r in csv.DictReader(f)}
    initial=Counter({i:math.ceil((round(float(r['on_hand_qty']))+overlay.get(r['item_id'],0))*fraction)
                     for i,r in enumerate(stock)})
    drafts=defaultdict(list)
    with (base/'inputs/outbound.csv').open(newline='') as f:
        for r in csv.DictReader(f):drafts[r['order_id']].append(r)
    epoch=min(dt.datetime.fromisoformat(rows[0]['order_drop_time']) for rows in drafts.values())
    orders=[]
    for order_id,(label,rows) in enumerate(sorted(drafts.items())):
        demand=Counter()
        for r in rows:demand[sku_ids[r['sku_id']]]+=int(r['order_qty'])
        release=round((dt.datetime.fromisoformat(rows[0]['order_drop_time'])-epoch).total_seconds()*1000)
        if spec['settings']['order'].get('release_all_at_start',False):release=0
        orders.append(dict(id=order_id,label=label,demand=demand,release=release))
    summary={};phases=defaultdict(dict);tail=[]
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
                            key=u.unpack()
                            if key in ('sim_time_secs','putaway_log','pick_log'):summary[key]=u.unpack()
                            else:u.skip()
                    elif key=='time_slices':
                        horizon=round(summary['sim_time_secs']*1000)
                        for _ in range(u.read_array_header()):
                            row={}
                            for _ in range(u.read_map_header()):
                                key=u.unpack()
                                if key in ('start_ms','station_diag','metrics'):row[key]=u.unpack()
                                else:u.skip()
                            for phase in row.get('station_diag',{}).get('order_phases',[]):
                                phases[phase['order_id']][phase['phase']]=phase['at']
                            if row['start_ms']>=horizon-10000:
                                tail.append(dict(start_ms=row['start_ms'],metrics={k:v for k,v in row['metrics'].items()
                                    if k in ('inventory_units','putaways_completed','putaway_presentations','picks_completed')}))
                    else:u.skip()
    horizon=round(summary['sim_time_secs']*1000)
    snapshots=[]
    for at in sorted({min(horizon,h*3_600_000) for h in (6,10,16,52)} | {horizon}):
        supply=initial.copy();prepared=Counter();waiting=[];unassigned=[];picked=0;completed=0
        for fill in summary['putaway_log']:
            if fill['sim_time_ms']<=at:supply[fill['sku']]+=fill['quantity']
        for order in orders:
            phase=phases[order['id']]
            if phase.get('PreparationStarted',at+1)<=at:prepared.update(order['demand'])
            if phase.get('FinalPickCompleted',at+1)<=at:completed+=1
        available=Counter({sku:supply[sku]-prepared[sku] for sku in supply.keys()|prepared.keys()})
        negative={sku:qty for sku,qty in available.items() if qty<0}
        if negative:raise ValueError(f'Negative unreserved supply at {at}: {list(negative.items())[:5]}')
        for order in orders:
            phase=phases[order['id']]
            if order['release']>at:continue
            if phase.get('Assigned',at+1)>at:unassigned.append(order)
            elif phase.get('PreparationStarted',at+1)>at:waiting.append(order)
        ready=[o for o in unassigned if all(available[sku]>=qty for sku,qty in o['demand'].items())]
        shortages=[]
        for order in waiting:
            missing=[dict(sku=sku,source_sku=stock[sku]['item_id'],need=qty,
                          unreserved=available[sku],ever_inducted=supply[sku]-initial[sku])
                     for sku,qty in order['demand'].items() if available[sku]<qty]
            shortages.append(dict(order_id=order['id'],source_order=order['label'],
                                  assigned_hours=phases[order['id']]['Assigned']/3_600_000,
                                  wait_hours=(at-phases[order['id']]['Assigned'])/3_600_000,missing=missing))
        picked=sum(r['quantity'] for r in summary['pick_log'] if r['sim_time_ms']<=at)
        snapshots.append(dict(hours=at/3_600_000,picks=picked,completed_orders=completed,
            assigned_waiting_for_stock=len(waiting),waiting_with_missing_stock=sum(bool(r['missing']) for r in shortages),
            waiting_for_never_inducted_sku=sum(any(s['ever_inducted']==0 and initial[s['sku']]==0 for s in r['missing']) for r in shortages),
            median_assignment_wait_hours=statistics.median(r['wait_hours'] for r in shortages) if shortages else None,
            released_unassigned=len(unassigned),individually_stock_ready_unassigned=len(ready),
            unreserved_units=sum(available.values()),missing_stock_examples=sorted(shortages,key=lambda r:-r['wait_hours'])[:8],
            ready_unassigned_examples=[dict(order_id=o['id'],source_order=o['label'],units=sum(o['demand'].values())) for o in ready[:8]]))
    future=[r for r in summary['putaway_log'] if r['sim_time_ms']>horizon]
    result=dict(profile=str(profile),horizon_hours=horizon/3_600_000,preload_units=sum(initial.values()),
                snapshots=snapshots,terminal_slices=tail,post_cutoff_fills=future,
                evidence='Reconstructed from frozen source IDs, timestamped fills and PreparationStarted allocation events; no simulation rerun or demand change.')
    save(folder/'backlog.json',result)
    return result


if __name__=='__main__':
    result=diagnose(BASE/sys.argv[1])
    print(json.dumps({**result,'snapshots':[{k:v for k,v in r.items() if not k.endswith('examples')} for r in result['snapshots']]},indent=2))
