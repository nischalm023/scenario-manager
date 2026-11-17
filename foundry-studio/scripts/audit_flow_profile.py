#!/usr/bin/env python3
"""Reconcile flow metrics and elapsed heatmap inputs without loading all playback.
ARCHIVE TOOL: the DESPROF v4 container this reads was retired with the
`.worldview` format. It still opens recordings made by binaries from before
that change; nothing writes the format any more.


Python 3.14 + msgpack. This checks recorded data, not the GPU's final picture.
"""

import argparse
from collections import Counter
from compression import zstd
import json
import math
from pathlib import Path
import struct

import msgpack

from audit_buffer_profile import FrameReader, extraction_rows, skip_bytes


def completion_journals(summary, extractions):
    """All recorded completions, including accepted work beyond the run cutoff."""
    fills = summary['putaway_log']
    fill_ids = {row['presentation_id'] for row in fills if row['presentation_id'] is not None}
    stores = [(row['sim_time_ms'] + row['total_cycle_ms'], 1)
              for row in summary['vtm_cycle_log']
              if row['direction'] == 'Return' and row['tote_source'] == 'Putaway'
              and row['presentation_id'] in fill_ids]
    journals = {
        'picks_completed': [(r['sim_time_ms'], r['quantity']) for r in summary['pick_log']],
        'putaways_completed': [(r['sim_time_ms'], r['quantity']) for r in fills],
        'putaway_presentations': [(r['sim_time_ms'], 1) for r in fills],
        'putaway_totes_stored': stores,
    }
    if extractions is not None:
        journals['order_extract_presentations'] = [(r['completed_at'], 1) for r in extractions]
        journals['order_extracted_units'] = [(r['completed_at'], r['extracted_units']) for r in extractions]
    return journals


def audit(path):
    summary = {}
    metadata = {}
    totals = Counter()
    diagnostics = Counter()
    traffic = Counter()
    errors = []
    initial_inventory = final_inventory = None
    last_bucket = None
    extractions = None
    extraction_columns = None
    finalized = False
    hourly = {}
    fields = {'sim_time_secs', 'total_picks', 'total_putaways', 'htm_idle_pct',
              'vtm_idle_pct', 'pick_log', 'putaway_log', 'vtm_cycle_log',
              'throughput_putaway_items_per_hour'}
    with path.open('rb') as raw:
        if raw.read(8) != b'DESPROF\x04':
            raise ValueError('Expected DESPROF v4')
        with zstd.open(raw, 'rb') as stream:
            while tag := stream.read(1):
                length = struct.unpack('<Q', stream.read(8))[0]
                if tag == b'\x02':
                    frame = msgpack.unpackb(stream.read(length), raw=False, strict_map_key=False)
                    if frame['stream_id'] == 'fulfillment.extractions':
                        extraction_columns, rows = extraction_rows(frame, extraction_columns)
                        if extractions is None:
                            extractions = []
                        extractions.extend(rows)
                    continue
                if tag == b'\x7f':
                    finalized = True
                if tag != b'\x7e':
                    skip_bytes(stream, length)
                    continue
                decoder = msgpack.Unpacker(FrameReader(stream, length), raw=False,
                                          strict_map_key=False, max_buffer_size=2**31)
                for _ in range(decoder.read_map_header()):
                    key = decoder.unpack()
                    if key == 'metadata':
                        metadata = decoder.unpack()
                    elif key == 'summary':
                        for _ in range(decoder.read_map_header()):
                            field = decoder.unpack()
                            if field in fields:
                                summary[field] = decoder.unpack()
                            else:
                                decoder.skip()
                    elif key == 'time_slices':
                        horizon = round(summary['sim_time_secs'] * 1000)
                        width = metadata['slice_width_ms']
                        for _ in range(decoder.read_array_header()):
                            row = decoder.unpack()
                            start = row['start_ms']
                            end = min(start + width, horizon)
                            if start > horizon:
                                continue
                            last_bucket = (start, start + width)
                            metrics = row['metrics']
                            for name, value in metrics.items():
                                if not math.isfinite(value):
                                    errors.append(f'nonfinite metric {name} at {start}')
                            for name in ('picks_completed', 'putaways_completed', 'putaway_totes_stored',
                                         'putaway_presentations', 'order_extract_presentations',
                                         'order_extracted_units'):
                                if name in metrics:
                                    totals[name] += metrics[name]
                                    hourly.setdefault(start // 3_600_000, Counter())[name] += metrics[name]
                            if 'inventory_units' in metrics:
                                if initial_inventory is None:
                                    # Stock is sampled after the slice's events.
                                    # Undo that first bucket to recover the preload.
                                    initial_inventory = (metrics['inventory_units']
                                                         - metrics.get('putaways_completed', 0)
                                                         + metrics.get('picks_completed', 0))
                                final_inventory = metrics['inventory_units']
                            for name, records in row.get('station_diag', {}).items():
                                if isinstance(records, list):
                                    diagnostics[name] += len(records)
                            for robot, loc in row['bot_locations'].items():
                                if loc['robot_type'] != 'Htm' or end <= start:
                                    continue
                                traffic['htm_slice_ms'] += end - start
                                spans = loc.get('motion')
                                if spans is None:
                                    traffic['legacy_htm_slices'] += 1
                                    continue
                                moving = 0
                                previous = start
                                for span in sorted(spans, key=lambda item: item['start_ms']):
                                    lo, hi = max(start, span['start_ms']), min(end, span['end_ms'])
                                    if hi <= lo:
                                        continue
                                    if lo < previous:
                                        errors.append(f'overlapping motion: HTM {robot} at {lo}')
                                    previous = hi
                                    moving += hi - lo
                                    if not math.isfinite(span['distance_mm']) or span['distance_mm'] <= 0:
                                        errors.append(f'invalid distance: HTM {robot} at {lo}')
                                traffic['canonical_moving_ms'] += moving
                                at = start
                                rendered_moving = 0
                                for step in loc['steps']:
                                    stop = at + step['delta_ms']
                                    duration = max(0, min(stop, end) - at)
                                    if step['kind'] in ('Move', 'Reverse'):
                                        rendered_moving += duration
                                    at = stop
                                traffic['playback_moving_ms'] += rendered_moving
                                traffic['absolute_motion_difference_ms'] += abs(moving - rendered_moving)
                    else:
                        decoder.skip()
    horizon = round(summary['sim_time_secs'] * 1000)
    journals = completion_journals(summary, extractions)
    expected = {name: sum(qty for at, qty in records if at <= horizon)
                for name, records in journals.items()}
    raw_totals = dict(totals)
    raw_final_inventory = final_inventory
    excluded = {}
    if last_bucket and last_bucket[0] <= horizon < last_bucket[1]:
        # Buckets retain accepted future work. Remove only timestamped events
        # after the cutoff within this bucket; events exactly at T still count.
        excluded = {name: sum(qty for at, qty in records if horizon < at < last_bucket[1])
                    for name, records in journals.items()}
        for name, qty in excluded.items():
            totals[name] -= qty
            if qty:
                hourly.setdefault(last_bucket[0] // 3_600_000, Counter())[name] -= qty
        if final_inventory is not None:
            final_inventory += excluded['picks_completed'] - excluded['putaways_completed']
    for name, value in expected.items():
        if totals[name] != value:
            errors.append(f'{name}: slices={totals[name]}, journal={value}')
    summary_differences = {}
    for field, count in [('total_picks', expected['picks_completed']),
                         ('total_putaways', expected['putaways_completed'])]:
        if summary[field] != count:
            summary_differences[field] = dict(reported=summary[field], within_cutoff=count)
            errors.append(f'{field}: summary={summary[field]}, within cutoff={count}')
    induction_rate = (expected['putaways_completed'] * 3600 / summary['sim_time_secs']
                      if summary['sim_time_secs'] else 0)
    if not math.isclose(summary['throughput_putaway_items_per_hour'], induction_rate):
        errors.append('summary induction rate does not use the within-cutoff completion count')
    expected_inventory = (None if initial_inventory is None else
                          initial_inventory + expected['putaways_completed'] - expected['picks_completed'])
    if final_inventory != expected_inventory:
        errors.append(f'inventory balance: observed={final_inventory}, expected={expected_inventory}')
    if not finalized:
        errors.append('profile not finalized')
    if extractions is None:
        errors.append('missing extraction journal; extraction counts cannot be reconciled')
    if traffic['legacy_htm_slices']:
        errors.append('missing canonical motion in HTM slices')
    if traffic['absolute_motion_difference_ms']:
        errors.append('canonical and playback moving durations differ')
    return dict(audit_version=2, profile=str(path), finalized=finalized, sim_hours=summary['sim_time_secs'] / 3600,
                flow_totals=dict(totals), flow_journal_counts=expected,
                raw_slice_totals=raw_totals, raw_final_inventory=raw_final_inventory,
                cutoff_bucket=dict(start_ms=last_bucket[0], end_ms=last_bucket[1],
                                   excluded_future_completions=excluded) if last_bucket else None,
                extraction_journal_checked=extractions is not None,
                summary_boundary_differences=summary_differences,
                hourly={hour: dict(values) for hour, values in hourly.items()},
                initial_inventory=initial_inventory, final_inventory=final_inventory,
                expected_inventory=expected_inventory,
                htm_idle_pct=summary['htm_idle_pct'], vtm_idle_pct=summary['vtm_idle_pct'],
                diagnostics=dict(diagnostics), traffic=dict(traffic), errors=errors[:50])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('profile', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    result = audit(args.profile)
    content = json.dumps(result, indent=2) + '\n'
    if args.output:
        args.output.write_text(content)
    else:
        print(content, end='')
    raise SystemExit(bool(result['errors']))
