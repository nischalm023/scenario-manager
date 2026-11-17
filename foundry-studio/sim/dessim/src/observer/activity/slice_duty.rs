//! Within `activity`, `slice_duty` derives each D30's duty in a slice once B
//! has passed the slice's end, from the robot's work as B stands across the
//! slice: the intervals built over it and what the open mark says of the rest.
//! A slice B never passed takes its duty from the final intervals when the
//! stream closes. Each slice's duty is derived once, and nothing books into it
//! afterwards.

use desgraph::RobotType;

use crate::model::SimTime;
use crate::observer::duty::book_slice_duty;
use crate::observer::state::SimObserverState;
use crate::observer::time_slice::passed_slice_end;

impl SimObserverState {
    /// Derive the D30 duty of each slice whose end B has passed.
    pub(super) fn derive_settled_duty(&mut self, floor: SimTime) {
        let width = self.config.slice_width();
        let mut index = self.activity.duty_upto;
        while index < self.slices().len() && passed_slice_end(floor, index, width) {
            self.derive_slice_duty(index);
            index += 1;
        }
        self.activity.duty_upto = index;
    }

    /// The D30 duty of every slice B never passed, from the final intervals.
    pub(super) fn derive_final_duty(&mut self) {
        for index in self.activity.duty_upto..self.slices().len() {
            self.derive_slice_duty(index);
        }
        self.activity.duty_upto = self.slices().len();
    }

    fn derive_slice_duty(&mut self, index: usize) {
        let width = self.config.slice_width_ms;
        let (from, to) = (SimTime::ms(index as u64 * width), SimTime::ms((index as u64 + 1) * width));
        let mut shares = Vec::new();
        for (&robot, &robot_type) in &self.output.robot_types {
            let Some(activity) = self.activity.robots.get(&robot).filter(|_| robot_type == RobotType::Htm) else {
                continue;
            };
            for work in activity.work_from(from).take_while(|work| work.start < to) {
                let (start, end) = (work.start.max(from), work.end.min(to));
                if end > start {
                    shares.push((robot, work.kind, end - start));
                }
            }
        }
        let scalars = &mut self.slice_to_finalize(index).scalars;
        for (robot, kind, share) in shares {
            book_slice_duty(scalars, robot, RobotType::Htm, kind, share);
        }
    }
}
