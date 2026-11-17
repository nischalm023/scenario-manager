"""Physical slot accounting must expose overbooking and respect the horizon."""

import unittest

from audit_buffer_profile import slot_occupancy


def visit(tote, level, start, end=None):
    row = {'tote_id': tote, 'rack_id': 17, 'rack_level': level, 'stored_at_ms': start}
    if end is not None:
        row['storage_pickup_at_ms'] = end
    return row


class SlotOccupancyTests(unittest.TestCase):
    def test_different_levels_are_independent_capacity(self):
        area, peak, overlaps = slot_occupancy([visit(1, 0, 10, 40), visit(2, 1, 20, 60)], 100)
        self.assertEqual((area, peak, overlaps), (70, 2, []))

    def test_pickup_and_deposit_at_same_time_do_not_overlap(self):
        area, peak, overlaps = slot_occupancy([visit(1, 0, 10, 40), visit(2, 0, 40, 60)], 100)
        self.assertEqual((area, peak, overlaps), (50, 1, []))

    def test_two_containers_in_one_slot_are_an_error_even_when_the_chart_counts_one(self):
        area, peak, overlaps = slot_occupancy([visit(1, 0, 10, 50), visit(2, 0, 20, 60)], 100)
        self.assertEqual((area, peak), (50, 1))
        self.assertEqual(overlaps, [{
            'at_ms': 20, 'rack_id': 17, 'rack_level': 0, 'incoming_tote_id': 2, 'occupants': [1],
        }])

    def test_unfinished_storage_counts_until_the_observed_horizon(self):
        area, peak, overlaps = slot_occupancy([visit(1, 0, 10), visit(2, 1, 40, 70)], 100)
        self.assertEqual((area, peak, overlaps), (120, 2, []))


if __name__ == '__main__':
    unittest.main()
