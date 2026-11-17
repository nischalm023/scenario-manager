#!/usr/bin/env python3
"""Compare two render dumps array by array and name the first field that differs.

A render dump is what desvis would have uploaded at one simulated instant (see
docs/designs/worldview-render-dump.md). Two dumps of the same run, one from
master's desvis and one from the branch's, must be byte identical; `cmp` would
say so too, but not which robot at which instant moved, so this walks the rows
with the layout the manifest states and says exactly that.

Differences are found in the raw bytes, never in decoded values: -0.0 and +0.0
are the same Python float and two NaNs with different bit patterns are neither
equal nor unequal, so a decoded comparison can pass over a real difference.
Once a field's bytes differ, the values are decoded for the message.

usage: dump_diff.py <dir-a> <dir-b>
Exit 0 when every array matches, 1 when any differs, 2 when the dumps are not
comparable (different instants, different arrays, different layouts).
"""
import json
import struct
import sys
from pathlib import Path

CODE = {'u8': 'B', 'u32': 'I', 'i32': 'i', 'u64': 'Q', 'f32': 'f'}


def load(folder):
    """The manifest, and its arrays as an ordered list of (key, entry) pairs.

    The order is the dump's upload order, which is part of what is compared,
    so the entries are never turned into a dictionary.
    """
    manifest = json.loads((folder / 'manifest.json').read_text())
    entries = [((entry['tick_ms'], entry['array']), entry) for entry in manifest['arrays']]
    seen = set()
    for key, _ in entries:
        if key in seen:
            raise ValueError(f'{folder}/manifest.json names t={key[0]} {key[1]} twice')
        seen.add(key)
    return manifest, entries


def field_slices(entry):
    """Each field's name, its byte range within a row, and its struct code."""
    return [(f['name'], f['offset'], f['offset'] + struct.calcsize(CODE[f['type']]), CODE[f['type']])
            for f in entry['fields']]


def decode(blob, at, code):
    (value,) = struct.unpack_from('<' + code, blob, at)
    return value


def first_difference(entry, a_blob, b_blob):
    """Where the two blobs first disagree, as a message, or None."""
    stride = entry['stride']
    fields = field_slices(entry)
    for row in range(entry['rows']):
        base = row * stride
        if a_blob[base:base + stride] == b_blob[base:base + stride]:
            continue
        for name, start, end, code in fields:
            a_bytes = a_blob[base + start:base + end]
            b_bytes = b_blob[base + start:base + end]
            if a_bytes == b_bytes:
                continue
            a_value = decode(a_blob, base + start, code)
            b_value = decode(b_blob, base + start, code)
            return (f'row {row} field {name}: {a_value} vs {b_value} '
                    f'({a_bytes.hex()} vs {b_bytes.hex()})')
        return f'row {row}: the row differs in a byte no field covers'
    return f'the blobs differ past row {entry["rows"]}'


def compare(a_dir, b_dir):
    a_manifest, a_entries = load(a_dir)
    b_manifest, b_entries = load(b_dir)
    if a_manifest['schema'] != b_manifest['schema']:
        print(f'schema {a_manifest["schema"]} vs {b_manifest["schema"]}', file=sys.stderr)
        return 2, []
    if a_manifest['ticks_ms'] != b_manifest['ticks_ms']:
        print('the dumps stand at different instants', file=sys.stderr)
        return 2, []
    if [key for key, _ in a_entries] != [key for key, _ in b_entries]:
        print('the dumps hold different arrays, or hold them in a different order', file=sys.stderr)
        return 2, []

    verdicts = []
    worst = 0
    for (key, a_entry), (_, b_entry) in zip(a_entries, b_entries):
        tick, name = key
        said = None
        # Shape first: equal bytes with an unequal row count or stride would
        # otherwise read as agreement.
        for what in ('rows', 'stride'):
            if a_entry[what] != b_entry[what]:
                said = f'{a_entry[what]} {what} vs {b_entry[what]}'
                worst = max(worst, 1)
                break
        if said is None and a_entry['fields'] != b_entry['fields']:
            said = 'the two sides state different layouts'
            worst = 2
        if said is None:
            a_blob = (a_dir / a_entry['file']).read_bytes()
            b_blob = (b_dir / b_entry['file']).read_bytes()
            expected = a_entry['rows'] * a_entry['stride']
            if len(a_blob) != expected or len(b_blob) != expected:
                said = f'{len(a_blob)} and {len(b_blob)} bytes for {expected}'
                worst = max(worst, 1)
            elif a_blob == b_blob:
                said = 'identical'
            else:
                said = first_difference(a_entry, a_blob, b_blob)
                worst = max(worst, 1)
        verdicts.append((tick, name, said))
    return worst, verdicts


def main():
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        outcome, verdicts = compare(Path(sys.argv[1]), Path(sys.argv[2]))
    except (OSError, ValueError, KeyError) as problem:
        print(f'the dumps are not comparable: {problem}', file=sys.stderr)
        return 2
    for tick, name, said in verdicts:
        print(f't={tick} {name}: {said}')
    return outcome


if __name__ == '__main__':
    sys.exit(main())
