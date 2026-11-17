//! Capacity over physical time. Coincident changes are combined so an end
//! and a new start at the same instant cannot create a fictitious peak.

use crate::model::{SimDuration, SimTime};

#[derive(Default)]
pub(super) struct Calendar {
    changes: Vec<(SimTime, isize)>,
}

impl Calendar {
    pub fn change_times(&self) -> impl Iterator<Item = SimTime> + '_ {
        self.changes.iter().map(|&(at, _)| at)
    }

    pub fn record(&mut self, at: SimTime, delta: isize) {
        match self.changes.binary_search_by_key(&at, |&(time, _)| time) {
            Ok(index) => {
                self.changes[index].1 += delta;
                if self.changes[index].1 == 0 {
                    self.changes.remove(index);
                }
            }
            Err(index) => self.changes.insert(index, (at, delta)),
        }
    }

    pub fn count_at(&self, at: SimTime) -> usize {
        self.changes.iter().take_while(|&&(time, _)| time <= at).map(|&(_, delta)| delta).sum::<isize>() as usize
    }

    pub fn open_after(&self, capacity: usize, after: SimTime) -> Option<SimTime> {
        let mut count = self.count_at(after) as isize;
        let mut candidate = after;
        for &(time, delta) in self.changes.iter().filter(|&&(time, _)| time > after) {
            if count >= capacity as isize {
                candidate = time;
            }
            count += delta;
        }
        (count < capacity as isize).then_some(candidate)
    }

    pub fn interval_after(&self, after: SimTime, duration: SimDuration) -> Option<SimTime> {
        let mut candidate = after;
        let mut count = self.count_at(after) as isize;
        for &(time, delta) in self.changes.iter().filter(|&&(time, _)| time > after) {
            if count == 0 && candidate.checked_add(duration)? <= time {
                return Some(candidate);
            }
            count += delta;
            if count == 0 {
                candidate = time;
            }
        }
        (count == 0).then_some(candidate).filter(|start| start.checked_add(duration).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::Calendar;
    use crate::model::SimTime;

    #[test]
    fn change_times_are_sorted_and_cancel_coincident_departures_and_arrivals() {
        let mut calendar = Calendar::default();
        calendar.record(SimTime::ms(20), 1);
        calendar.record(SimTime::ms(30), -1);
        calendar.record(SimTime::ms(10), 1);
        calendar.record(SimTime::ms(20), -1);
        calendar.record(SimTime::ms(5), 1);
        calendar.record(SimTime::ms(5), -1);

        assert_eq!(calendar.change_times().collect::<Vec<_>>(), vec![SimTime::ms(10), SimTime::ms(30)]);
        assert_eq!(calendar.count_at(SimTime::ms(9)), 0);
        assert_eq!(calendar.count_at(SimTime::ms(10)), 1);
        assert_eq!(calendar.count_at(SimTime::ms(20)), 1);
        assert_eq!(calendar.count_at(SimTime::ms(30)), 0);
    }
}
