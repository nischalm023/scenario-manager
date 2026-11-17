//! Within `scenarios`, `seal` is what spec 08's execution and runner routes take from spec 07 (sections 3.5 and
//! 3.8; brief I1.09 calls them): `seal`, inside the execution's transaction, re-reads the version an experiment was
//! prepared on and seals it at the experiment's revision, refusing a version edited since (`plan-stale`); and
//! `sealed_version`, the version a runner reads, never an open one.

use sqlx::{Error as SqlxError, PgConnection};
use studio_protocol::{Problem, ProblemType, Revision, Ulid, VersionNumber, VersionRead, VersionState};
use thiserror::Error;

use crate::problem::{ProblemAnswer, internal};

use super::reads::as_read;
use super::rows;

/// What a submission's seal did: sealed the open version now, or found it sealed at the experiment's revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sealed {
    Now,
    Already,
}

#[derive(Debug, Error)]
pub enum SealError {
    #[error("there is no version {version} of the scenario {scenario}")]
    Missing { scenario: Ulid, version: u32 },
    #[error("the scenario {0} was deleted")]
    Deleted(Ulid),
    #[error(
        "the scenario changed after these runs were prepared: version {version} is at revision {now}, not {planned}"
    )]
    PlanStale { version: u32, planned: u32, now: u32 },
    #[error(transparent)]
    Database(#[from] SqlxError),
}

impl SealError {
    pub fn answer(&self, instance: &str) -> ProblemAnswer {
        let type_ = match self {
            Self::Missing { .. } => ProblemType::NotFound,
            Self::Deleted(_) => ProblemType::ScenarioDeleted,
            Self::PlanStale { .. } => ProblemType::PlanStale,
            Self::Database(fault) => return internal("seal the scenario's version", fault, instance),
        };
        Problem::new(type_, format!("{}.", capitalised(&self.to_string()))).at(instance).into()
    }
}

/// Seal version `version` of `scenario` for `experiment`, prepared at `revision`, inside the caller's transaction:
/// the version's row is locked and re-read, an open version at that revision is sealed, a sealed one at it stays as
/// it is, and any other revision is `plan-stale`, so nothing runs.
pub async fn seal(
    conn: &mut PgConnection,
    scenario: &Ulid,
    version: VersionNumber,
    revision: Revision,
    experiment: &Ulid,
) -> Result<Sealed, SealError> {
    let row = rows::scenario(conn, scenario, true).await?;
    let missing = || SealError::Missing { scenario: scenario.clone(), version: version.0 };
    let row = row.ok_or_else(missing)?;
    if row.deleted_at.is_some() {
        return Err(SealError::Deleted(scenario.clone()));
    }
    let record = rows::version(conn, scenario, version, true).await?.ok_or_else(missing)?;
    if record.revision != revision {
        return Err(SealError::PlanStale { version: version.0, planned: revision.0, now: record.revision.0 });
    }
    match record.state {
        VersionState::Sealed => Ok(Sealed::Already),
        VersionState::Open => {
            rows::seal(conn, scenario, version, experiment).await?;
            Ok(Sealed::Now)
        }
    }
}

/// The sealed version a runner reads; `None` for a version that is open or does not exist.
pub async fn sealed_version(
    conn: &mut PgConnection,
    scenario: &Ulid,
    version: VersionNumber,
) -> Result<Option<VersionRead>, SqlxError> {
    let record = rows::version(conn, scenario, version, false).await?;
    Ok(record.filter(|record| record.state == VersionState::Sealed).map(as_read))
}

fn capitalised(sentence: &str) -> String {
    let mut chars = sentence.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}
