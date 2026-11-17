#!/usr/bin/env python3
"""Build a separate, offline variant atlas from retained DESPROF recordings.

Uses the streaming traffic_report binary; never starts a simulation or writes
report.html. Cached exports live beside each profile. The generated HTML embeds
its data, styles and chart renderer, so colleagues need only the HTML file.
"""
# ARCHIVE TOOL: the DESPROF v4 container this parses was retired with the
# `.worldview` format. It still opens recordings made by binaries from before
# that change; nothing writes the format any more.
import argparse
import datetime as dt
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
DEFAULT_BASE = REPO.parent / 'notes/walmart-matrix-2026-09-14'
DEFAULT_OUTPUT = REPO / 'docs/easyloop/overnight-2026-09-14/variants.html'


def read(path):
    return json.loads(path.read_text())


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def validate(series, flow, initial):
    assert series['htm_fleet_size'] > 0
    fields = {'picks': 'picks_completed', 'inducted_units': 'putaways_completed',
              'filled_totes': 'putaway_presentations', 'stored_totes': 'putaway_totes_stored'}
    for field, audit_field in fields.items():
        actual = sum(row[field] for row in series['bins'])
        assert actual == flow['flow_totals'][audit_field], (field, actual, flow['flow_totals'][audit_field])
    assert initial + series['racks'][-1]['net_inventory_units'] == flow['final_inventory']
    for row in series['racks']:
        assert row['capacity'] > 0
        assert row['occupied'] == row['stocked'] + row['empty'] + row['unknown'] <= row['capacity']
    assert sum(b['wait']['elapsed_wait_ms'] for b in series['bins']) == series['totals']['elapsed_wait_ms']
    assert series['totals']['elapsed_wait_ms'] <= series['htm_fleet_size'] * series['bins'][-1]['end_ms']


def export_case(base, case, extractor, refresh):
    folder = base / case['id']
    profiles = list(folder.glob('*.desprofile'))
    if len(profiles) != 1:
        raise ValueError(f"Expected one recording in {folder}, found {len(profiles)}")
    profile = profiles[0]
    signature = {'profile_sha256': digest(profile), 'spec_sha256': digest(folder/'spec.json'),
                 'extractor_sha256': digest(extractor), 'bin_minutes': 5}
    cache = folder / 'variant-series.json'
    exported = read(cache) if cache.exists() else None
    if refresh or exported is None or exported.get('export_signature') != signature:
        print(f"Extracting {case['id']} ({profile.stat().st_size / 1e6:.0f} MB)", flush=True)
        with (folder/'variant-series.log').open('w') as log:
            subprocess.run([str(extractor), str(profile), str(folder/'spec.json'), str(case['hours']),
                            str(cache), '--series-minutes', '5'], cwd=REPO, check=True, stdout=log, stderr=subprocess.STDOUT)
        exported = read(cache)
        exported['export_signature'] = signature
        cache.write_text(json.dumps(exported, separators=(',', ':')))
    flow = read(folder/'flow-audit.json')
    if flow['errors']:
        raise ValueError(f"Flow audit failed for {case['id']}: {flow['errors']}")
    series = exported['series']
    validate(series, flow, int(flow['initial_inventory']))
    facts = read(folder/'report-facts.json')
    series.update(initial_units=int(flow['initial_inventory']), final_units=int(flow['final_inventory']),
                  profile=profile.name, profile_sha256=signature['profile_sha256'],
                  recorded_hours=exported['recorded_end_ms']/3_600_000,
                  common=facts['windows']['common'], whole=facts['windows']['whole'],
                  audit_errors=flow['errors'], profile_metadata=facts['metadata'])
    return series


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', type=Path, default=DEFAULT_BASE)
    parser.add_argument('--output', type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument('--original-report', type=Path, default=DEFAULT_OUTPUT.with_name('report.html'),
                        help='Single-page report whose hash is recorded and whose contents stay unchanged')
    parser.add_argument('--extractor', type=Path, default=REPO/'target/release/traffic_report')
    parser.add_argument('--refresh', action='store_true', help='Re-extract profile series even if cached')
    args = parser.parse_args()
    original = args.original_report
    protected_hash = digest(original)
    if args.output.resolve() == original.resolve():
        raise ValueError('The companion must not overwrite report.html')
    manifest = read(args.base/'manifest.json')
    source = read(args.base/'source-audit.json')
    cases = []
    for case in manifest['cases']:
        row = dict(case)
        row.pop('args')
        row['state'] = read(args.base/case['id']/'state.json')
        row['config'] = read(args.base/case['id']/'spec.json')
        row['preload'] = source['preload'][{0:'empty', .5:'partial', 1:'full'}[case['preload_fraction']]]
        if case['alias_of']:
            row['status'] = 'equivalent'
        elif row['state']['status'] == 'finished':
            row['status'] = 'recorded'
            row['series'] = export_case(args.base, case, args.extractor, args.refresh)
        elif row['state']['status'] == 'failed':
            with (args.base/case['id']/'run.log').open('rb') as log:
                preflight = b'Inventory cannot fulfill all orders' in log.read(1_000_000)
            if not preflight:
                raise ValueError(f"Unexpected failure: {case['id']}")
            row['status'] = 'preflight'
        else:
            raise ValueError(f"No final outcome: {case['id']}")
        cases.append(row)
    data = {'generated_at':dt.datetime.now().astimezone().isoformat(), 'manifest':manifest,
            'source':source, 'cases':cases, 'original_report_sha256':protected_hash}
    embedded = json.dumps(data, separators=(',', ':'), ensure_ascii=False).replace('<', '\\u003c')
    css = (HERE/'variants.css').read_text()
    js = (HERE/'variants.js').read_text()
    document = f'''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Walmart · Variant time series</title><style>{css}</style></head>
<body><a class="skip" href="#content">Skip to variant</a>
<header><div><span class="eyebrow">Walmart DFW6 · 14 September 2026</span><h1>Variant time series</h1></div>
<div class="header-links"><a href="report.html">Read the original report ↗</a><button id="download">Download chart data</button></div></header>
<div class="layout"><aside><p class="eyebrow">18 variants · 12 recordings</p><label for="search">Find a variant</label>
<input id="search" type="search" placeholder="e.g. partial, source, lead…"><nav id="variants" aria-label="Variants"></nav></aside>
<main id="content" tabindex="-1"></main></div><div id="tooltip" role="status" hidden></div>
<noscript>This offline HTML uses JavaScript to select variants and render the embedded chart data. Enable JavaScript to view the pages. The original report remains available separately.</noscript>
<script id="report-data" type="application/json">{embedded}</script><script>{js}</script></body></html>'''
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(document)
    assert digest(original) == protected_hash, 'Original report changed'
    validation = {'variants':len(cases), 'recordings':sum(c['status']=='recorded' for c in cases),
                  'original_report_sha256':protected_hash, 'companion_sha256':digest(args.output),
                  'checks':'Completion totals, inventory balance, rack capacities, wait conservation and unchanged original report passed.'}
    (args.base/'companion-data-validation.json').write_text(json.dumps(validation, indent=2))
    print(f"Wrote {args.output} ({args.output.stat().st_size/1e6:.2f} MB); original report unchanged.")


if __name__ == '__main__':
    main()
