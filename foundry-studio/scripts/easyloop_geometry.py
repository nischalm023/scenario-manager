#!/usr/bin/env python3
"""Declared EasyLoop cells and links shared by the minimal and DFW6 authors.

Local millimetres follow the DWG block: +x right and +y away from the operator.
Belt extents come from that drawing. The traffic pattern and presentation phases
are Daniel's September 7 rulings. Presentation positions are authored beside the
inventory chutes. Turns have a 700 mm centreline radius: approximately 2.21 m
outer diameter with a conservative 700 mm square robot envelope. Each
presentation has its own six-location waiting block and entry/egress pair
(Daniel's September 12 layout ruling).
Presentation cells are declared directly; circular links contain the bends.
"""
from math import cos, sin, radians, hypot, ceil

DECK_W = 610.0
V_WEST_C, V_EAST_C = -840.0, 0.0
V_Y_N = -1424.0
V_Y_S_WEST, V_Y_S_EAST = -5564.4, -4795.4
H_SOUTH_C, H_NORTH_C, H_X_E = -6659.4, -5891.65, 5729.4
X_MIN, X_MAX = -1145.0, 6034.4
PORT_SPACING_MM = V_EAST_C - V_WEST_C
CELL_SIZE_MM = 695.0
LANE_PITCH_X_MM, LANE_PITCH_Y_MM = 770.0, 850.0  # surrounding site floor, not a machine lattice
WAITING_COUNT = 6
WAITING_PITCH_MM = 700.0
CONVEYOR_CAPACITY = 8
TURN_BODY_MM = 700.0  # conservative square around the displayed HTM, not a surveyed body dimension
TURN_RADIUS_MM = WAITING_PITCH_MM
TURN_OUTER_RADIUS_MM = ceil(hypot(TURN_RADIUS_MM + TURN_BODY_MM / 2, TURN_BODY_MM / 2) * 10) / 10
PRESENTATION_PHASE = [135.0, 45.0]
# Daniel moved each order presentation 500 mm north and 500 mm outward, beside
# the inventory chutes. Keep these declared positions fixed as turn size changes.
ORDER_POSITION = [(1903.1, -7455.4), (4183.9, -7455.4)]
TURN_CENTERS = [
    (round(x + TURN_RADIUS_MM * cos(radians(phase)), 1),
     round(y + TURN_RADIUS_MM * sin(radians(phase)), 1))
    for (x, y), phase in zip(ORDER_POSITION, PRESENTATION_PHASE)
]
# Straight lanes meet the circle tangentially, with one full-size waiting
# column between them. The apron columns follow these boundary positions.
BOUNDARY_COLUMNS = {
    0: round(TURN_CENTERS[0][0] - TURN_RADIUS_MM, 1),
    1: round(TURN_CENTERS[0][0] + TURN_RADIUS_MM, 1),
    5: round(TURN_CENTERS[1][0] - TURN_RADIUS_MM, 1),
    6: round(TURN_CENTERS[1][0] + TURN_RADIUS_MM, 1),
}
ORDER_COLUMNS = dict(BOUNDARY_COLUMNS)
WAITING_COLUMNS = [(ORDER_COLUMNS[0] + ORDER_COLUMNS[1]) / 2,
                   (ORDER_COLUMNS[5] + ORDER_COLUMNS[6]) / 2]
ORDER_TOP_Y = V_Y_N
LOOP_TRANSIT_SECS, TIME_PER_PICK_SECS = 45, 9


def turn_point(side, phase):
    cx, cy = TURN_CENTERS[side]
    a = radians(phase)
    return (round(cx - TURN_RADIUS_MM * cos(a), 1), round(cy - TURN_RADIUS_MM * sin(a), 1))


PRESENTATION_HEADING = {side: (phase - 90.0) % 360 for side, phase in enumerate(PRESENTATION_PHASE)}


def heading(side, mirrored=False):
    # Reflection swaps physical sides. Reverse local circulation too, so both
    # physical U-turns still enter on their left and return on their right.
    return (PRESENTATION_HEADING[side] + (180 if mirrored else 0)) % 360


def cells(waiting_count=WAITING_COUNT):
    if waiting_count < 1:
        raise ValueError('a waiting block needs at least one location')
    # Extra rows extend north; the dispatch end remains fixed above the belts.
    top = ORDER_TOP_Y + max(0, waiting_count - WAITING_COUNT) * WAITING_PITCH_MM
    out = [('port:dropoff', (V_WEST_C, V_Y_N), 'dropoff'),
           ('port:collect', (V_EAST_C, V_Y_N), 'collect')]
    rows = [V_Y_N - (i + 1) * WAITING_PITCH_MM for i in range(CONVEYOR_CAPACITY // 2)]
    points = [(V_WEST_C, y) for y in rows] + [(V_EAST_C, y) for y in reversed(rows)]
    out += [(f'inventory:{i}', xy, 'lane') for i, xy in enumerate(points)]
    for col, x in BOUNDARY_COLUMNS.items():
        out.append((f'order:{col}/0', (x, top), 'doorway'))
    for row in range(1, waiting_count + 1):
        for col, x in ORDER_COLUMNS.items():
            out.append((f'order:{col}/{row}', (x, top - row * WAITING_PITCH_MM), 'lane'))
        for side, x in enumerate(WAITING_COLUMNS):
            out.append((f'wait:{side}/{row - 1}', (x, top - row * WAITING_PITCH_MM), 'waiting'))
    for side in (0, 1):
        out.append((f'present:{side}', ORDER_POSITION[side], 'presentation'))
    return out


def inventory_drive():
    return ['port:dropoff'] + [f'inventory:{i}' for i in range(CONVEYOR_CAPACITY)] + ['port:collect']


def lanes(mirrored=False):
    # Order circulation is stated by its links. Putting the bottom travel cell
    # in an exclusive approach queue would cut off its waiting-cell branch.
    return [('lane:inventory', inventory_drive()[1:],
            'One-way under-conveyor queue through eight holding locations to collection. No early exit after dropoff.')]


def links(waiting_count=WAITING_COUNT, mirrored=False):
    """(name, pairs, bidirectional, optional circle) for each explicit stitch."""
    out = [('inventory queue', list(zip(inventory_drive(), inventory_drive()[1:])), False, None)]
    def run(name, path, bidirectional=False, curve=None):
        pairs = list(zip(path, path[1:]))
        if mirrored and not bidirectional:
            pairs = [(b, a) for a, b in reversed(pairs)]
        if curve is not None:
            curve = dict(curve, clockwise=mirrored)
            if mirrored:
                curve = {({'start': 'end', 'end': 'start'}.get(k, k)): v for k, v in curve.items()}
        out.append((name, pairs, bidirectional, curve))
    last = waiting_count
    for col in (0, 1, 5, 6):
        path = [f'order:{col}/{row}' for row in range(waiting_count + 1)]
        if col in (1, 6):
            path.reverse()
        if len(path) > 1:
            run(f'order column {col}', path)
    for side, (entry, returning) in enumerate(((0, 1), (5, 6))):
        if mirrored:
            entry, returning = returning, entry
        for row in range(waiting_count):
            wait = f'wait:{side}/{row}'
            # Both travel lanes can enter a wait; only the descending lane can
            # receive its dispatch. Reflection changes which lane descends.
            out.append((f'waiting dispatch {side}/{row}',
                        [(f'order:{entry}/{row + 1}', wait)], True, None))
            out.append((f'waiting return {side}/{row}',
                        [(f'order:{returning}/{row + 1}', wait)], False, None))
    for side, (entry, returning) in enumerate(((0, 1), (5, 6))):
        run(f'order turn {side}', [f'order:{entry}/{last}', f'present:{side}'],
            curve={'center': TURN_CENTERS[side], 'start': turn_point(side, 0)})
        run(f'order turn {side} departure', [f'present:{side}', f'order:{returning}/{last}'],
            curve={'center': TURN_CENTERS[side], 'end': turn_point(side, 180)})
    return out


def arcs():
    return {name: pairs for name, pairs, _bidir, _curve in links()}


def doors(mirrored=False):
    out = [('port:dropoff', 'entry'), ('port:collect', 'exit')]
    out += [(f'order:{col}/0', 'entry' if ((col in (0, 5)) != mirrored) else 'exit') for col in (0, 1, 5, 6)]
    return out


def envelope():
    points = [xy for _, xy, _ in cells()]
    half = CELL_SIZE_MM / 2
    min_x, max_x = min(X_MIN, min(x for x, _ in points) - half), max(X_MAX, max(x for x, _ in points) + half)
    min_y = min(cy - TURN_OUTER_RADIUS_MM for _, cy in TURN_CENTERS)
    max_y = max(y for _, y in points) + half
    return min_x, min_y, max_x-min_x, max_y-min_y


def clearances():
    return [('under-conveyor HTM holding locations', CONVEYOR_CAPACITY),
            ('waiting locations per independent presentation block (configurable)', WAITING_COUNT),
            ('turn centre radius with straight entry and return lanes', TURN_RADIUS_MM)]


def check_layout(table=None):
    table = cells() if table is None else table
    for i, (a, (ax, ay), _) in enumerate(table):
        for b, (bx, by), _ in table[i+1:]:
            assert abs(ax-bx) >= CELL_SIZE_MM or abs(ay-by) >= CELL_SIZE_MM, f'{a} overlaps {b}'
    at = {handle: xy for handle, xy, _ in table}
    for i in range(CONVEYOR_CAPACITY):
        x, y = at[f'inventory:{i}']
        assert x in (V_WEST_C, V_EAST_C) and V_Y_S_EAST < y < V_Y_N
    return table


def counted():
    table = check_layout()
    by_role = {role: sum(r == role for _, _, r in table) for _, _, role in table}
    return {'cells': len(table), 'ports': 2, 'presentation': 2, 'waiting': by_role['waiting'],
            'lane steps': by_role['lane'], 'doorways': by_role['doorway'],
            'arcs': sum(len(pairs) for pairs in arcs().values()), 'doors': len(doors())}


if __name__ == '__main__':
    for item in check_layout():
        print(item)
    print(counted())
