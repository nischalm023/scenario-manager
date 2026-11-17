"""Run endpoints include exact-time completions, never later accepted work."""
# ARCHIVE TOOL: the DESPROF v4 container this parses was retired with the
# `.worldview` format. It still opens recordings made by binaries from before
# that change; nothing writes the format any more.

from compression import zstd
from pathlib import Path
import struct
import tempfile
import unittest

import msgpack

from audit_flow_profile import audit


def recording(path, horizon, fills, picks=(), extractions=(), stores=(), corrupt_units=0):
    width = 10_000
    fill_rows = [dict(sim_time_ms=at, quantity=qty, presentation_id=i)
                 for i, (at, qty) in enumerate(fills)]
    pick_rows = [dict(sim_time_ms=at, quantity=qty) for at, qty in picks]
    summary = dict(sim_time_secs=horizon / 1000,
                   total_picks=sum(qty for at, qty in picks if at <= horizon),
                   total_putaways=sum(qty for at, qty in fills if at <= horizon),
                   throughput_putaway_items_per_hour=(sum(qty for at, qty in fills if at <= horizon)
                                                      * 3_600_000 / horizon if horizon else 0),
                   htm_idle_pct=1, vtm_idle_pct=1,
                   pick_log=pick_rows, putaway_log=fill_rows,
                   vtm_cycle_log=[dict(sim_time_ms=at-100, total_cycle_ms=100,
                                       direction='Return', tote_source='Putaway', presentation_id=fill_id)
                                  for at, fill_id in stores])
    slices = []
    for start in range(0, horizon + width, width):
        end = start + width
        metrics = dict(picks_completed=sum(q for t, q in picks if start <= t < end),
                       putaways_completed=sum(q for t, q in fills if start <= t < end),
                       putaway_presentations=sum(start <= t < end for t, _ in fills),
                       putaway_totes_stored=sum(start <= t < end for t, _ in stores),
                       order_extract_presentations=sum(start <= t < end for t, _ in extractions),
                       order_extracted_units=sum(q for t, q in extractions if start <= t < end),
                       inventory_units=50 + sum(q for t, q in fills if t < end)
                       - sum(q for t, q in picks if t < end))
        if start <= horizon < end:
            metrics['putaways_completed'] += corrupt_units
            metrics['inventory_units'] += corrupt_units
        slices.append(dict(start_ms=start, metrics=metrics, bot_locations={}))
    rows = b''.join(msgpack.packb([at, qty]) for at, qty in extractions)
    stream = dict(stream_id='fulfillment.extractions',
                  descriptor=dict(columns=[dict(name='completed_at'), dict(name='extracted_units')]),
                  entries=dict(encoding='RowsV1', bytes=rows, entry_count=len(extractions)))
    document = dict(metadata=dict(slice_width_ms=width), summary=summary, time_slices=slices)
    frames = b''
    for tag, obj in [(b'\x02', stream), (b'\x7e', document), (b'\x7f', {})]:
        payload = msgpack.packb(obj, use_bin_type=True)
        frames += tag + struct.pack('<Q', len(payload)) + payload
    path.write_bytes(b'DESPROF\x04' + zstd.compress(frames))


class CutoffTests(unittest.TestCase):
    def run_audit(self, horizon, fills, **kwargs):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'test.desprofile'
            recording(path, horizon, fills, **kwargs)
            return audit(path)

    def test_exact_boundary_pick_stays_and_later_fill_is_excluded(self):
        result = self.run_audit(10_000, [(9999, 5), (11_000, 6), (22_000, 7)],
                                picks=[(10_000, 2)])
        self.assertEqual(result['errors'], [])
        self.assertEqual(result['flow_totals']['putaways_completed'], 5)
        self.assertEqual(result['flow_totals']['picks_completed'], 2)
        self.assertEqual(result['raw_final_inventory'], 59)
        self.assertEqual(result['final_inventory'], 53)
        self.assertEqual(result['cutoff_bucket']['excluded_future_completions']['putaways_completed'], 6)

    def test_exact_boundary_fill_counts_even_with_zero_elapsed_terminal_bucket(self):
        result = self.run_audit(10_000, [(10_000, 6)])
        self.assertEqual(result['errors'], [])
        self.assertEqual(result['flow_totals']['putaways_completed'], 6)
        self.assertEqual(result['final_inventory'], 56)

    def test_partial_bucket_counts_only_completed_picks_fills_and_extractions(self):
        result = self.run_audit(15_000, [(12_000, 5), (17_000, 6)],
                                picks=[(14_000, 2), (18_000, 3)],
                                extractions=[(15_000, 2), (19_000, 3)])
        self.assertEqual(result['errors'], [])
        self.assertTrue(result['extraction_journal_checked'])
        self.assertEqual(result['flow_totals']['order_extract_presentations'], 1)
        self.assertEqual(result['flow_totals']['order_extracted_units'], 2)
        self.assertEqual(result['flow_totals']['picks_completed'], 2)
        self.assertEqual(result['final_inventory'], 53)

    def test_correction_does_not_hide_unexplained_extra_units(self):
        result = self.run_audit(10_000, [(9999, 5), (11_000, 6)], corrupt_units=3)
        self.assertEqual(len(result['errors']), 2)
        self.assertTrue(any('putaways_completed: slices=8' in e for e in result['errors']))
        self.assertTrue(any('inventory balance' in e for e in result['errors']))

    def test_filled_tote_rack_stores_use_their_own_completion_times(self):
        result = self.run_audit(10_000, [(1000, 5), (2000, 6)],
                                stores=[(10_000, 0), (11_000, 1)])
        self.assertEqual(result['errors'], [])
        self.assertEqual(result['flow_totals']['putaways_completed'], 11)
        self.assertEqual(result['flow_totals']['putaway_totes_stored'], 1)

    def test_zero_horizon_can_contain_a_completion_at_zero(self):
        result = self.run_audit(0, [(0, 2), (1, 6)])
        self.assertEqual(result['errors'], [])
        self.assertEqual(result['initial_inventory'], 50)
        self.assertEqual(result['final_inventory'], 52)


if __name__ == '__main__':
    unittest.main()
