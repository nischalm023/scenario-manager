#!/usr/bin/env python3
"""Freeze and run the September 14 workload matrix, with bounded concurrency.

One supervisor owns the job queue and process records. Its simulation children
use PTYs solely for progress output; no GUI is launched. Completed profiles are
audited by a separate worker so reporting does not delay another simulation.
"""
import argparse
import datetime as dt
import errno
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import selectors
import shutil
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]
BASE = REPO.parent / 'notes/walmart-matrix-2026-09-14'


def now():
    return dt.datetime.now().astimezone().isoformat()


def save(path, data):
    temporary = path.with_suffix('.pending')
    temporary.write_text(json.dumps(data, indent=2) + '\n')
    temporary.replace(path)


def digest(path):
    return hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()


def processes():
    lines = subprocess.check_output(['ps', '-axo', 'pid,ppid,rss,etime,command'], text=True).splitlines()[1:]
    result = []
    for line in lines:
        fields = line.strip().split(None, 4)
        if len(fields) != 5:
            continue
        pid, parent, rss, elapsed, command = fields
        executable = command.split(' ', 1)[0]
        if Path(executable).name.startswith(('desvis', 'Walmart Simulation')):
            result.append(dict(pid=int(pid), parent=int(parent), rss_mb=int(rss)/1024,
                               elapsed=elapsed, command=command, headless='--headless' in command))
    return result


def memory():
    raw = subprocess.check_output(['vm_stat'], text=True)
    page = int(re.search(r'page size of (\d+)', raw)[1])
    fields = {key: int(value) for key, value in re.findall(r'^([^:]+):\s+(\d+)\.', raw, re.M)}
    # Conservative reclaimable estimate: inactive pages capped by file-backed
    # pages. Do not count all inactive anonymous memory as launch headroom.
    available = (fields['Pages free'] + fields['Pages speculative']
                 + min(fields['Pages inactive'], fields['File-backed pages'])) * page
    return dict(available_gb=available/2**30, compressor_gb=fields['Pages occupied by compressor']*page/2**30,
                swapouts=fields['Swapouts'], pageouts=fields['Pageouts'])


def prepare():
    BASE.mkdir(parents=True, exist_ok=True)
    if (BASE/'manifest.json').exists():
        raise SystemExit('Matrix already exists; use --supervise to resume it.')
    fixtures = REPO/'sim/dessim/tests/fixtures'
    inputs = BASE/'inputs'
    inputs.mkdir()
    for name in ('walmart_dfw6.json', 'walmart_demo_stock_top_up.csv', 'walmart-data/onhand.csv', 'walmart-data/outbound.csv'):
        shutil.copy2(fixtures/name, inputs/Path(name).name)
    binary = BASE/'desvis-iteration-38'
    shutil.copy2(REPO/'target/release/desvis', binary)
    template = json.loads((fixtures/'walmart_dfw6_putaway_partial_spec.json').read_text())
    template['world'] = str(inputs/'walmart_dfw6.json')
    for key, name in [('onhand_csv','onhand.csv'), ('outbound_csv','outbound.csv'),
                      ('stock_top_up_csv','walmart_demo_stock_top_up.csv')]:
        template['scenario'][key] = str(inputs/name)
    template['settings']['num_htms'] = 230
    commit = subprocess.check_output(['git','rev-parse','HEAD'], cwd=REPO, text=True).strip()
    cases = []

    def add(preload, putaway, timing, sequence='order_release', lead=None, hours=16, alias=None):
        name = f'{preload}-{putaway}-{timing}' + (f'-{sequence}' if putaway=='on' else '')
        if lead is not None:
            name += '-lead1h'
        fraction = {'full':1.,'partial':.5,'empty':0.}[preload]
        spec = json.loads(json.dumps(template))
        roots = spec['composition']['tasks']
        fill = next(t for t in roots if t['type']=='FillInventory')
        fill['config'].update(order_trays=128,preload_fraction=fraction)
        if putaway=='off':
            roots[:] = [t for t in roots if t['type'] not in ('PutawaySupplyDemand','PutawayInduction')]
        else:
            induction = next(t for t in roots if t['type']=='PutawayInduction')
            induction['config']['sequence'] = sequence
            if lead is not None:
                induction['config']['lead_time_secs'] = lead
        spec['settings']['order']['release_all_at_start'] = timing=='t0'
        spec['settings']['sim_duration'] = dict(secs=hours*3600,nanos=0)
        folder = BASE/name
        folder.mkdir()
        save(folder/'spec.json',spec)
        case = dict(id=name,preload_fraction=fraction,putaway=putaway,timing=timing,sequence=sequence if putaway=='on' else None,
                    lead_time_secs=lead,hours=hours,common_horizon_hours=16,iteration=38+len(cases),
                    spec=str(folder/'spec.json'),spec_sha256=digest(folder/'spec.json'),alias_of=alias)
        case['args'] = [str(binary),case['spec'],'--headless','--progress','--simtime',f'{hours}h',
                        '--wall-timeout','6h' if hours>16 else '3h','--allow-incomplete','--output-dir',str(folder),
                        '--name',name,'--label',f"Walmart iteration {case['iteration']:02d} | matrix {name} | {hours}h"]
        save(folder/'state.json',dict(status='equivalent_no_inbound' if alias else 'queued',updated_at=now(),alias_of=alias))
        cases.append(case)

    # Start useful independent extremes before the policy controls and negative cases.
    add('full','off','source',hours=52)
    add('empty','on','t0')
    add('partial','on','t0')
    add('full','off','t0')
    add('empty','on','source',hours=52)
    add('partial','on','source')
    for preload in ('empty','partial'):
        for timing in ('t0','source'):
            add(preload,'on',timing,'sku_demand')
    for preload in ('partial','empty'):
        for timing in ('t0','source'):
            add(preload,'off',timing)
    for timing in ('t0','source'):
        add('full','on',timing,alias=f'full-off-{timing}')
    for preload in ('empty','partial'):
        add(preload,'on','source',lead=3600)
    manifest = dict(created_at=now(),source_commit=commit,binary=str(binary),binary_sha256=digest(binary),
                    inputs={p.name:digest(p) for p in inputs.iterdir()},htms=230,vtms=48,order_trays=128,
                    pick_seconds=10,queue_factor=3,putaway_presentations_hour_station=60,induction_max_units_tote=6,
                    buffer_seconds=3600,seed=0,collision_avoidance=True,common_horizon_hours=16,cases=cases)
    save(BASE/'manifest.json',manifest)
    print(json.dumps(dict(manifest=str(BASE/'manifest.json'),cases=len(cases),source=commit)))


def supervise():
    import fcntl
    lock=(BASE/'supervisor.lock').open('a')
    fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
    manifest=json.loads((BASE/'manifest.json').read_text())
    selector=selectors.DefaultSelector()
    active={}
    last_check=0
    while True:
        for key,_ in selector.select(timeout=1):
            job=active[key.fd]
            try:data=os.read(key.fd,65536)
            except OSError as error:
                if error.errno!=errno.EIO:raise
                data=b''
            if data:
                job['log'].write(data);job['log'].flush()
            else:
                selector.unregister(key.fd);os.close(key.fd)
                job['log'].close()
                code=job['proc'].wait()
                state=job['state'];state.update(status='finished' if code==0 else 'failed',exit_code=code,finished_at=now())
                save(job['folder']/'state.json',state)
                active.pop(key.fd)
        if time.monotonic()-last_check<10:
            continue
        last_check=time.monotonic()
        procs=processes();mem=memory()
        for job in active.values():
            own=next((p for p in procs if p['pid']==job['proc'].pid),None)
            if own:
                job['state']['rss_mb']=own['rss_mb']
                job['state']['peak_sampled_rss_mb']=max(own['rss_mb'],job['state'].get('peak_sampled_rss_mb',0))
                save(job['folder']/'state.json',job['state'])
        # An existing run from another supervisor still counts against the cap.
        count=sum(p['headless'] for p in procs)
        states={c['id']:json.loads((BASE/c['id']/'state.json').read_text()) for c in manifest['cases']}
        pending=[c for c in manifest['cases'] if states[c['id']]['status']=='queued']
        save(BASE/'supervisor.json',dict(pid=os.getpid(),updated_at=now(),memory=mem,desvis=procs,
                                        queued=len(pending),active=len(active),cases={k:v['status'] for k,v in states.items()}))
        headed=sum(not p['headless'] for p in procs)
        if headed>2:
            print('More than two headed processes exist; no new launches until reconciled.',flush=True)
        if pending and count<4 and headed<=2 and mem['available_gb']>=22 and mem['compressor_gb']<12:
            case=pending[0];folder=BASE/case['id'];master,slave=pty.openpty()
            log=(folder/'run.log').open('ab')
            proc=subprocess.Popen(case['args'],cwd=REPO,stdin=subprocess.DEVNULL,stdout=slave,stderr=slave,start_new_session=True)
            os.close(slave)
            state=dict(status='running',pid=proc.pid,started_at=now(),args=case['args'])
            save(folder/'state.json',state)
            active[master]=dict(proc=proc,log=log,state=state,folder=folder)
            selector.register(master,selectors.EVENT_READ)
            print(now(),'started',case['id'],proc.pid,flush=True)
        if not pending and not active:
            break
    save(BASE/'supervisor-finished.json',dict(finished_at=now(),pid=os.getpid()))


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--prepare',action='store_true')
    parser.add_argument('--supervise',action='store_true')
    args=parser.parse_args()
    if args.prepare:prepare()
    if args.supervise:supervise()
