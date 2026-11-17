#!/usr/bin/env python3
"""Refresh declared EasyLoop geometry in the two maintained fixtures.

The full-map apron follows the moved doors, keeping independent aligned
approaches. Other floor/fleet entities and machine frames remain untouched.
The two authoring scripts supply cells, queues and internal links from
easyloop_geometry.py. Use --check to verify, or --apply to update the fixtures.
"""
import argparse
import copy
import json
import re
import tempfile
import uuid
from pathlib import Path

import gen_minimal_easyloop as minimal
import reauthor_dfw6_easyloops as dfw6
import dfw6_floor_alignment as floor


def update(text, authored, names, boundary_ids):
    original = json.loads(text)
    spans, _ = dfw6.entity_spans(text)

    def internal(entity):
        name = entity.get('name', '')
        return entity['id'] not in boundary_ids and any(
            name.startswith(f'{machine}: ') or name.startswith(f'stitch:{machine} ')
            for machine in names)

    desired = {e['id']: e for e in authored if internal(e)}
    edits = []
    changed = set()
    for entity in original['entities']:
        envelope = entity.get('name') in names and 'go.easyloop' in entity['components']
        if not internal(entity) and not envelope:
            continue
        ident = entity['id']
        if envelope:
            candidate = copy.deepcopy(entity)
            frame = entity['components']['frame']['transform_to_parent']
            x, y, width, height = dfw6.geom.envelope()
            corners = [(x, y), (x + width, y + height)]
            transformed = [[frame['translation'][i] + frame['scale'][i] * dfw6.mm(p[i])
                            for i in range(2)] for p in corners]
            lo = [min(p[i] for p in transformed) for i in range(2)]
            hi = [max(p[i] for p in transformed) for i in range(2)]
            candidate['components']['region'].update(origin=lo, size=[hi[i] - lo[i] for i in range(2)])
        else:
            candidate = desired.pop(ident, None)
        if candidate is not None:
            candidate = copy.deepcopy(candidate)
            # Preserve existing explanatory comments; this is a geometry edit.
            for key, value in entity['components'].items():
                replacement = candidate['components'].get(key)
                if key == '$comment':
                    candidate['components'][key] = value
                elif isinstance(value, dict) and isinstance(replacement, dict) and '$comment' in value:
                    replacement['$comment'] = value['$comment']
            if candidate == entity:
                continue
        a, b = spans[ident]
        edits.append((a, b, dfw6.render(candidate).lstrip() if candidate else None, ident))
        changed.add(ident)
    result = dfw6.splice(text, edits, list(desired.values()), False)
    result_spans, _ = dfw6.entity_spans(result)
    for entity in original['entities']:
        if entity['id'] not in changed:
            a, b = spans[entity['id']]
            c, d = result_spans[entity['id']]
            assert text[a:b] == result[c:d], f'changed unrelated entity {entity["id"]}'
    return result


def realign_apron(text):
    """Retile the existing apron around its 26 explicit door connections."""
    original = {e['id']: e for e in json.loads(text)['entities']}
    entities = copy.deepcopy(original)
    doors = []
    for entity in entities.values():
        stitch = entity['components'].get('stitch', {})
        if stitch.get('rule') != 'explicit' or stitch.get('b') != floor.APRON:
            continue
        pose = entities[stitch['a']]['components']['pose']
        x = pose['position'][0]
        if pose['frame'] != floor.FLOOR:
            frame = entities[pose['frame']]['components']['frame']['transform_to_parent']
            x = frame['translation'][0] + frame['scale'][0] * x
        doors.append((entity, x))
    assert len(doors) == 26
    apron = entities[floor.APRON]['components']
    tpl = entities[apron['stamp']['template']]
    derives = tpl['components']['template']['entities'][0]['components']['tiling']['derives']
    columns = floor.apron_columns(sorted(x for _, x in doors), apron['region']['size'][0])
    tpl['components']['template'] = floor.columns_template(tpl['name'], columns, derives)['components']['template']
    for entity, x in doors:
        col = next(i for i, (start, width) in enumerate(columns) if 2 * start + width == 2 * x)
        for pair in entity['components']['stitch']['pairs']:
            for i, endpoint in enumerate(pair):
                if isinstance(endpoint, dict) and endpoint.get('generator') == floor.APRON:
                    pair[i] = floor.address(floor.APRON, f'column-{col}', 0, 0)
    # Every reference to an apron column must be among the remapped doors.
    door_ids = {entity['id'] for entity, _ in doors}
    for entity in original.values():
        if entity['id'] not in door_ids:
            assert f'"generator": "{floor.APRON}"' not in json.dumps(entity), entity['id']
    spans, _ = dfw6.entity_spans(text)
    changed = {key for key in original if original[key] != entities[key]}
    edits = [(*spans[key], dfw6.render(entities[key]).lstrip(), key) for key in changed]
    result = dfw6.splice(text, edits, [], False)
    after, _ = dfw6.entity_spans(result)
    for key in original.keys() - changed:
        a, b = spans[key]
        c, d = after[key]
        assert text[a:b] == result[c:d], f'changed unrelated apron neighbour {key}'
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--check', action='store_true')
    mode.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    minimal_path = minimal.OUT
    with tempfile.TemporaryDirectory() as directory:
        minimal.OUT = Path(directory) / 'minimal.json'
        minimal.main()
        minimal_entities = json.loads(minimal.OUT.read_text())['entities']
    for path, names in [(minimal_path, ['easyloop 1']), (dfw6.DOC, [m[0] for m in dfw6.MACHINES])]:
        text = path.read_text()
        if path == minimal_path:
            authored = minimal_entities
            boundaries = {minimal.eid(f'easyloop 1/stitch:doorway:{handle}') for handle, _ in dfw6.geom.doors()}
        else:
            ns = uuid.UUID(json.loads(text)['doc']['id'])
            authored = [e for name, _, x, mirrored, ext in dfw6.MACHINES
                        for e in dfw6.machine_entities(ns, name, x, mirrored, ext)[0]]
            boundaries = {dfw6.eid(ns, f'easyloop:{n}:stitch:doorway:{handle}')
                          for n in range(1, 5) for handle, _ in dfw6.geom.doors()}
        result = update(text, authored, names, boundaries)
        if path == dfw6.DOC:
            result = realign_apron(result)
        if path == minimal_path:
            source_comment = json.loads(text)['$comment']
            updated_comment = re.sub(r'envelope, \d+ declared cells',
                                     f'envelope, {dfw6.geom.counted()["cells"]} declared cells', source_comment)
            updated_comment = updated_comment.replace('three lanes that name the place each serves', 'an inventory queue')
            result = result.replace(json.dumps(source_comment), json.dumps(updated_comment), 1)
        if args.check:
            assert result == text, f'{path.name} needs --apply'
        elif result != text:
            path.write_text(result)
        print(f'{path.name}: {"updated" if result != text else "current"}; unrelated entities preserved byte for byte')


if __name__ == '__main__':
    main()
