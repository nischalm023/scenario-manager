#!/usr/bin/env python3
"""Audit physical order-buffer visits in a DESPROF v4 recording.
ARCHIVE TOOL: the DESPROF v4 container this reads was retired with the
`.worldview` format. It still opens recordings made by binaries from before
that change; nothing writes the format any more.


Requires Python 3.14 and msgpack. Supply buffer aisle IDs from the recorded
map, not from their apparent occupancy. Example for the current DFW6 map:
  python3 scripts/audit_buffer_profile.py run.desprofile --zones 21,22,23,24,25,26,27,28

The two occupancy calculations use independent recorded facts: VTM storage
operations and aisle-state transitions. Unfinished visits are right-censored
at the observation horizon; they remain in the slot-time integral.
"""

import argparse
from collections import defaultdict
from compression import zstd
import json
import math
from pathlib import Path
import statistics
import struct

import msgpack


SUMMARY_FIELDS = {
    'sim_time_secs', 'total_picks', 'htm_idle_pct', 'vtm_idle_pct',
    'completed_order_log', 'pick_log', 'vtm_cycle_log', 'vtm_operation_log',
    'aisle_state_log', 'rack_zone_capacity',
}


def skip_bytes(stream, remaining):
    while remaining:
        chunk = stream.read(min(remaining, 1_048_576))
        if not chunk:
            raise ValueError('Truncated profile frame')
        remaining -= len(chunk)


class FrameReader:
    """Keep MessagePack read-ahead inside this frame's declared byte extent."""

    def __init__(self, stream, length):
        self.stream = stream
        self.remaining = length

    def read(self, size=-1):
        size = self.remaining if size < 0 else min(size, self.remaining)
        chunk = self.stream.read(size)
        self.remaining -= len(chunk)
        return chunk


def extraction_rows(frame, columns):
    """Decode one typed extraction-stream frame; retain columns across chunks."""
    if frame['descriptor']:
        columns = [c['name'] for c in frame['descriptor']['columns']]
    entries = frame['entries']
    if not columns or entries['encoding'] != 'RowsV1':
        raise ValueError('Unsupported extraction stream')
    decoder = msgpack.Unpacker(raw=False)
    decoder.feed(entries['bytes'])
    rows = [dict(zip(columns, row, strict=True)) for row in decoder]
    if len(rows) != entries['entry_count']:
        raise ValueError('Incomplete extraction rows')
    return columns, rows


def read_profile(path):
    result = {'extractions': [], 'rack_arrivals': {}, 'profile_finalized': False}
    extraction_stream_present = False
    columns = None
    with path.open('rb') as raw:
        if raw.read(8) != b'DESPROF\x04':
            raise ValueError('Expected DESPROF v4')
        with zstd.open(raw, 'rb') as stream:
            while tag := stream.read(1):
                length = struct.unpack('<Q', stream.read(8))[0]
                if tag == b'\x02':
                    frame = msgpack.unpackb(stream.read(length), raw=False, strict_map_key=False)
                    if frame['stream_id'] != 'fulfillment.extractions':
                        continue
                    extraction_stream_present = True
                    columns, rows = extraction_rows(frame, columns)
                    result['extractions'].extend(rows)
                elif tag == b'\x7e':
                    frame = FrameReader(stream, length)
                    decoder = msgpack.Unpacker(frame, raw=False, strict_map_key=False, max_buffer_size=2**31)
                    for _ in range(decoder.read_map_header()):
                        key = decoder.unpack()
                        if key == 'summary':
                            summary = {}
                            for _ in range(decoder.read_map_header()):
                                field = decoder.unpack()
                                if field in SUMMARY_FIELDS:
                                    summary[field] = decoder.unpack()
                                else:
                                    decoder.skip()
                            result['summary'] = summary
                        elif key in ('metadata', 'sim_config'):
                            result[key] = decoder.unpack()
                        elif key == 'time_slices':
                            # Older operation rows omitted rack level. Read only
                            # tote arrivals, not the large robot playback arrays.
                            for _ in range(decoder.read_array_header()):
                                row = {}
                                for _ in range(decoder.read_map_header()):
                                    field = decoder.unpack()
                                    if field in ('start_ms', 'tote_state'):
                                        row[field] = decoder.unpack()
                                    else:
                                        decoder.skip()
                                for offset, tote, state in row.get('tote_state', {}).get('transitions', []):
                                    if isinstance(state, dict) and 'InRack' in state:
                                        result['rack_arrivals'][(row['start_ms'] + offset, tote)] = state['InRack']['level']
                        else:
                            decoder.skip()
                    if decoder.tell() != length:
                        raise ValueError('Legacy document does not match its frame extent')
                elif tag == b'\x7f':
                    skip_bytes(stream, length)
                    result['profile_finalized'] = True
                else:
                    skip_bytes(stream, length)
    if 'summary' not in result:
        raise ValueError('No profile summary')
    if not extraction_stream_present:
        raise ValueError('No extraction stream; missing data cannot establish zero extractions')
    return result


def describe_ms(values):
    values = sorted(values)
    if not values:
        return {'count': 0}
    return {
        'count': len(values),
        'mean_seconds': statistics.mean(values) / 1000,
        'median_seconds': statistics.median(values) / 1000,
        'p90_seconds': values[math.ceil(len(values) * .90) - 1] / 1000,
        'max_seconds': values[-1] / 1000,
    }


def aisle_integral(records, zones, horizon):
    by_time = defaultdict(list)
    capacity = {}
    for row in records:
        if row['zone_id'] in zones and row['sim_time_ms'] <= horizon:
            by_time[row['sim_time_ms']].append(row)
            old = capacity.setdefault(row['zone_id'], row['storage_capacity'])
            if old != row['storage_capacity']:
                raise ValueError('Changing capacity needs a capacity-time denominator')
    current = {zone: 0 for zone in zones}
    area = peak = previous = 0
    for at, rows in sorted(by_time.items()):
        area += sum(current.values()) * (at - previous)
        for row in rows:
            current[row['zone_id']] = row['storage_occupied']
        peak = max(peak, sum(current.values()))
        previous = at
    area += sum(current.values()) * (horizon - previous)
    return area, peak, sum(current.values()), sum(capacity.values())


def slot_occupancy(visits, horizon):
    """Count physical slots separately from containers, exposing double booking."""
    events = []
    for visit in visits:
        if visit['rack_level'] is None:
            continue
        slot = (visit['rack_id'], visit['rack_level'])
        start = visit['stored_at_ms']
        end = visit.get('storage_pickup_at_ms', horizon)
        if end > start:
            events.extend([(start, 1, slot, visit['tote_id']), (end, 0, slot, visit['tote_id'])])
    occupied = defaultdict(set)
    area = previous = peak = count = 0
    overlaps = []
    for at, arriving, slot, tote in sorted(events):
        area += count * (at - previous)
        previous = at
        if arriving:
            if occupied[slot]:
                overlaps.append({
                    'at_ms': at, 'rack_id': slot[0], 'rack_level': slot[1],
                    'incoming_tote_id': tote, 'occupants': sorted(occupied[slot]),
                })
            else:
                count += 1
            occupied[slot].add(tote)
            peak = max(peak, count)
        else:
            occupied[slot].remove(tote)
            if not occupied[slot]:
                count -= 1
    return area, peak, overlaps


def audit(data, zones):
    summary = data['summary']
    horizon = round(summary['sim_time_secs'] * 1000)
    if horizon <= 0:
        raise ValueError('Need a positive observed duration')
    capacity = sum(cap for zone, cap in summary['rack_zone_capacity'] if zone in zones)
    if not capacity:
        raise ValueError('No recorded capacity for the specified buffer aisles')
    dwell = round(data['sim_config']['order']['buffer_dwell_secs'] * 1000)
    errors = []
    active = {}
    visits = []
    extraction_by_trip = defaultdict(list)
    extractions = [r for r in data['extractions'] if r['completed_at'] <= horizon]
    for row in extractions:
        extraction_by_trip[row['order_trip_id']].append(row)
    operations = sorted(
        (r for r in summary['vtm_operation_log']
         if r['zone_id'] in zones and r['sim_time_ms'] <= horizon
         and r['kind'] in ('DropStorage', 'PickupStorage')),
        key=lambda r: (r['sim_time_ms'], r['kind'] != 'PickupStorage', r['tote_id']),
    )
    for row in operations:
        tote, at = row['tote_id'], row['sim_time_ms']
        if row['kind'] == 'DropStorage':
            if tote in active:
                errors.append(f'Duplicate storage of tote {tote}')
            active[tote] = {
                'tote_id': tote, 'zone_id': row['zone_id'], 'stored_at_ms': at,
                'eligible_at_ms': at + dwell,
                'rack_id': row['rack_id'],
                'rack_level': row.get('rack_level') if row.get('rack_level') is not None
                else data.get('rack_arrivals', {}).get((at, tote)),
            }
        else:
            visit = active.pop(tote, None)
            if visit is None:
                errors.append(f'Tote {tote} retrieved without a recorded buffer deposit; possible initial stock')
                continue
            visit['storage_pickup_at_ms'] = at
            if row['rack_id'] != visit['rack_id'] or (
                row.get('rack_level') is not None and row['rack_level'] != visit['rack_level']
            ):
                errors.append(f'Tote {tote} retrieved from a different rack slot than its deposit')
            if at < visit['eligible_at_ms']:
                errors.append(f'Tote {tote} retrieved before its configured eligibility')
            candidates = [r for r in extraction_by_trip[str(row['presentation_id'])] if r['completed_at'] >= at]
            if len(candidates) == 1:
                visit['extracted_at_ms'] = candidates[0]['completed_at']
            elif len(candidates) > 1:
                errors.append(f'Ambiguous extraction trip for tote {tote}')
            visits.append(visit)
    visits.extend(active.values())
    visits.sort(key=lambda v: (v['stored_at_ms'], v['tote_id']))
    completed = [v for v in visits if 'storage_pickup_at_ms' in v]
    residence = [v['storage_pickup_at_ms'] - v['stored_at_ms'] for v in completed]
    censored = [horizon - v['stored_at_ms'] for v in active.values()]
    visit_area = sum(residence) + sum(censored)
    missing_levels = sum(v['rack_level'] is None for v in visits)
    if missing_levels:
        errors.append(f'{missing_levels} buffer visits lack rack level; slot exclusivity cannot be verified')
    unique_slot_area, unique_slot_peak, overlaps = slot_occupancy(visits, horizon)
    if overlaps:
        errors.append(f'{len(overlaps)} buffer deposits overlap an existing occupant of the same rack slot')
    state_area, peak, end_count, state_capacity = aisle_integral(summary['aisle_state_log'], zones, horizon)
    if not missing_levels and state_area != unique_slot_area:
        errors.append(f'Aisle occupancy differs from distinct occupied rack slots by {state_area - unique_slot_area} slot-ms')
    if state_capacity != capacity:
        errors.append('Aisle-state and rack capacity denominators disagree')
    if state_area != visit_area:
        errors.append(f'Physical occupancy integrals disagree by {state_area - visit_area} slot-ms')
    if end_count != len(active):
        errors.append('End-of-window aisle occupancy differs from unmatched deposits')
    hours = horizon / 3_600_000
    picks = [r for r in summary['pick_log'] if r['sim_time_ms'] <= horizon]
    counts = []
    for start in range(0, horizon, 3_600_000):
        end = min(horizon, start + 3_600_000)
        counts.append({
            'start_ms': start, 'end_ms': end,
            'picks': sum(r['quantity'] for r in picks if start <= r['sim_time_ms'] < end),
            'extractions': sum(start <= r['completed_at'] < end for r in extractions),
            'buffer_deposits': sum(start <= v['stored_at_ms'] < end for v in visits),
            'buffer_retrievals': sum(start <= v['storage_pickup_at_ms'] < end for v in completed),
        })
    return {
        'metadata': data['metadata'], 'horizon_ms': horizon, 'buffer_zones': sorted(zones),
        'profile_finalized': data.get('profile_finalized', False),
        'buffer_dwell_ms': dwell, 'capacity_slots': capacity,
        'picks': sum(r['quantity'] for r in picks),
        'picks_per_hour': sum(r['quantity'] for r in picks) / hours,
        'extractions': len(extractions), 'buffer_deposits': len(visits),
        'buffer_retrievals': len(completed), 'still_stored': len(active),
        'mean_occupied_slots': state_area / horizon, 'peak_occupied_slots': peak,
        'mean_fill_percent': 100 * state_area / (horizon * capacity),
        'slot_time_from_visits_ms': visit_area, 'slot_time_from_aisle_states_ms': state_area,
        'slot_time_from_distinct_slots_ms': unique_slot_area if not missing_levels else None,
        'peak_distinct_slots': unique_slot_peak if not missing_levels else None,
        'overlapping_deposits': len(overlaps), 'slot_overlaps': overlaps,
        'residence_completed': describe_ms(residence),
        'stored_age_at_horizon': describe_ms(censored),
        'eligibility_to_storage_pickup_including_handling': describe_ms([
            v['storage_pickup_at_ms'] - v['eligible_at_ms'] for v in completed]),
        'storage_pickup_to_extraction_including_transport_and_service': describe_ms([
            v['extracted_at_ms'] - v['storage_pickup_at_ms'] for v in completed if 'extracted_at_ms' in v]),
        'retrieved_without_matching_extraction_by_horizon': sum('extracted_at_ms' not in v for v in completed),
        'htm_idle_percent': 100 * summary['htm_idle_pct'],
        'vtm_idle_percent': 100 * summary['vtm_idle_pct'],
        'hourly': counts, 'errors': errors, 'visits': visits,
    }


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('profile', type=Path)
    parser.add_argument('--zones', required=True, help='Comma-separated order-buffer aisle IDs from this map')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    result = audit(read_profile(args.profile), {int(z) for z in args.zones.split(',')})
    result['profile'] = str(args.profile.resolve())
    output = args.output or args.profile.with_name('buffer-audit.json')
    output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: value for key, value in result.items() if key not in ('visits', 'hourly', 'slot_overlaps')}, indent=2))
    print(f'Saved {output}')
    if result['errors']:
        raise SystemExit(2)
