//! Delay a root factory until its declared setup tasks complete. Construction
//! itself is synchronous and runs once before the root makes any proposals.

use std::cell::RefCell;
use std::fmt;

use async_trait::async_trait;
use tokio::sync::OnceCell;

use super::{TaskContext, invalid, task_name};
use crate::error::PlanError;
use crate::scheduler::task::proposal::ProposalId;
use crate::scheduler::task::{SchedCtx, Task, TaskDependency};

pub(crate) struct Deferred<T: Task> {
    context: RefCell<Option<TaskContext>>,
    task: OnceCell<T>,
    after: Vec<TaskDependency>,
}

impl<T: Task> Deferred<T> {
    pub fn new(context: TaskContext, after: Vec<TaskDependency>) -> Self {
        Self { context: RefCell::new(Some(context)), task: OnceCell::new(), after }
    }
}

impl<T: Task> fmt::Display for Deferred<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.task.get() {
            Some(task) => task.fmt(f),
            None => f.write_str(task_name::<T>()),
        }
    }
}

#[async_trait(?Send)]
impl<T: Task> Task for Deferred<T> {
    type Proposal = T::Proposal;
    type Schedule = T::Schedule;

    async fn propose(&self, owner: ProposalId) -> Result<Self::Proposal, PlanError> {
        let task = self
            .task
            .get_or_try_init(|| async {
                for dependency in &self.after {
                    dependency.wait().await?;
                }
                let context = self
                    .context
                    .borrow_mut()
                    .take()
                    .ok_or_else(|| invalid(format!("{} constructed twice", task_name::<T>())))?;
                let task = T::create(&context)?;
                context.finish()?;
                Ok::<T, PlanError>(task)
            })
            .await?;
        task.propose(owner).await
    }

    async fn schedule(&mut self, proposal: Self::Proposal, ctx: SchedCtx) -> Result<Self::Schedule, PlanError> {
        let task = self
            .task
            .get_mut()
            .ok_or_else(|| invalid(format!("{} scheduled before construction", task_name::<T>())))?;
        task.schedule(proposal, ctx).await
    }
}
