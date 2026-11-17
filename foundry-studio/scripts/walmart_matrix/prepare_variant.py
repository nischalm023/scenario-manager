#!/usr/bin/env python3
"""Prepare a portable Walmart review spec from the measured baseline controls.

This writes configuration only. The printed Cargo command launches a headed
simulation; no simulation is started by this script.
"""
import argparse
import json
import math
import os
from pathlib import Path
import shlex

REPO = Path(__file__).resolve().parents[2]
FIXTURES = REPO / 'sim/dessim/tests/fixtures'
FRACTIONS = {'partial': 0.5, 'full': 1.0, 'empty': 0.0}


def variant(preload, putaway, timing, sequence, lead_secs, hours):
    if not math.isfinite(hours) or hours <= 0 or round(hours * 3_600_000) == 0:
        raise ValueError('Hours must specify a positive duration of at least one millisecond')
    if lead_secs is not None:
        if not math.isfinite(lead_secs) or lead_secs < 0:
            raise ValueError('Induction lead must be a finite, nonnegative number of seconds')
        if (putaway, timing, sequence) != ('on', 'source', 'order_release'):
            raise ValueError('Induction lead requires putaway on, source order timing and order_release sequencing')
    spec = json.loads((FIXTURES / 'walmart_dfw6_baseline_partial_spec.json').read_text())
    tasks = spec['composition']['tasks']
    next(t for t in tasks if t['type'] == 'FillInventory')['config']['preload_fraction'] = FRACTIONS[preload]
    if putaway == 'off':
        tasks[:] = [t for t in tasks if t['type'] not in (
            'PutawaySupplyDemand', 'PutawayInduction')]
    else:
        induction = next(t for t in tasks if t['type'] == 'PutawayInduction')['config']
        induction['sequence'] = sequence
        induction.pop('lead_time_secs', None)
        if lead_secs is not None:
            induction['lead_time_secs'] = lead_secs
    spec['settings']['order']['release_all_at_start'] = timing == 't0'
    seconds, milliseconds = divmod(round(hours * 3_600_000), 1000)
    spec['settings']['sim_duration'] = {'secs': seconds, 'nanos': milliseconds * 1_000_000}
    return spec


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.ArgumentDefaultsHelpFormatter)
    parser.add_argument('--preload', choices=FRACTIONS, default='partial')
    parser.add_argument('--putaway', choices=('on', 'off'), default='on')
    parser.add_argument('--order-timing', choices=('t0', 'source'), default='t0')
    parser.add_argument('--induction-sequence', choices=('order_release', 'sku_demand'), default='order_release')
    parser.add_argument('--induction-lead-secs', type=float,
                        help='Make demand lots eligible this many seconds before source order release; omitted means all lots available at T0')
    parser.add_argument('--hours', type=float, default=16)
    parser.add_argument('--output', type=Path, default=REPO / 'target/walmart-variant.json')
    args = parser.parse_args()
    try:
        spec = variant(args.preload, args.putaway, args.order_timing,
                       args.induction_sequence, args.induction_lead_secs, args.hours)
    except ValueError as error:
        parser.error(str(error))
    output = args.output.resolve()
    if output in (FIXTURES / 'walmart_dfw6_baseline_partial_spec.json',
                  FIXTURES / 'walmart_dfw6_baseline_full_spec.json'):
        parser.error('Choose a generated output path; preserve the measured baseline presets')
    spec['world'] = os.path.relpath(FIXTURES / spec['world'], output.parent)
    for field in ('onhand_csv', 'outbound_csv', 'stock_top_up_csv'):
        spec['scenario'][field] = os.path.relpath(FIXTURES / spec['scenario'][field], output.parent)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(spec, indent=2) + '\n')
    label = f'Walmart | {args.preload} preload | putaway {args.putaway} | {args.order_timing} orders | {args.hours:g}h'
    if args.putaway == 'on':
        label += f' | {args.induction_sequence}'
    if args.induction_lead_secs is not None:
        label += f' | induction lead {args.induction_lead_secs:g}s'
    print(f'Wrote {output}\nRun from the repository root:')
    print(shlex.join(['cargo', 'run', '--release', '--locked', '-p', 'desvis', '--', str(output), '--run', '--label', label]))


if __name__ == '__main__':
    main()
