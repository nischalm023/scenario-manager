//! Validate explicit root completion dependencies and produce a stable
//! topological registration order. A perpetual root cannot be a prerequisite.

use rustc_hash::FxHashMap;

use super::{Definition, RootTask, invalid};
use crate::error::PlanError;
use crate::scheduler::Scheduler;
use crate::scheduler::task::TaskDependency;
use crate::task::registered;

impl Definition {
    pub(crate) fn ordered_roots(&self) -> Result<Vec<&RootTask>, PlanError> {
        let by_name: FxHashMap<_, _> = self.tasks.iter().map(|task| (task.name(), task)).collect();
        let mut states = FxHashMap::default();
        let mut ordered = Vec::new();
        for task in &self.tasks {
            visit(task, &by_name, &mut states, &mut ordered)?;
        }
        Ok(ordered)
    }

    pub(crate) fn spawn(&self, scheduler: &mut Scheduler) -> Result<(), PlanError> {
        let mut completed: FxHashMap<&str, TaskDependency> = FxHashMap::default();
        for task in self.ordered_roots()? {
            let after = task.after.iter().map(|name| completed[name.as_str()].clone()).collect();
            let completion = registered(task.name())?.spawn_after(scheduler, task.config.clone(), after)?;
            completed.insert(task.name(), completion);
        }
        Ok(())
    }
}

fn visit<'a>(
    task: &'a RootTask,
    by_name: &FxHashMap<&str, &'a RootTask>,
    states: &mut FxHashMap<&'a str, bool>,
    ordered: &mut Vec<&'a RootTask>,
) -> Result<(), PlanError> {
    match states.get(task.name()) {
        Some(true) => return Ok(()),
        Some(false) => return Err(invalid(format!("root dependency cycle at {}", task.name()))),
        None => {}
    }
    states.insert(task.name(), false);
    for name in &task.after {
        let predecessor = by_name
            .get(name.as_str())
            .ok_or_else(|| invalid(format!("{} depends on missing root {name}", task.name())))?;
        if !registered(predecessor.name())?.holds_completion() {
            return Err(invalid(format!("{} cannot wait for perpetual root {name}", task.name())));
        }
        visit(predecessor, by_name, states, ordered)?;
    }
    states.insert(task.name(), true);
    ordered.push(task);
    Ok(())
}
