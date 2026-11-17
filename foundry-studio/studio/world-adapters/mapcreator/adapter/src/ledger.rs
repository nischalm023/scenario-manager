//! Within the Map Creator adapter, `ledger` is **the import's account of
//! itself**: what it carried and where it put it, where each identity came
//! from, what the core protocol still cannot state, and — the part that decides
//! whether an ingest goes ahead — every field family the export carries that no
//! disposition covers.
//!
//! The one-way importer's ledger answered "how much of a real site is the new
//! format ready for", and listed what it had no home for. The round-trip crate
//! this adapter was ported from asked a narrower question with a sharper edge,
//! **is anything lost?**, and answered it by comparing the dump its export
//! rebuilt with the dump it started from. A module has no moment at which it
//! holds both: an ingest is handed the files and never writes them, a projection
//! writes them and is never handed them. So the comparison lives in the
//! round-trip laws, which run the module's two directions one after the other,
//! and what stays here is what the import itself can account for.
//!
//! A module also has no channel for a ledger: an ingest answers with changes or
//! with a sentence. So a ledger that does not pass is a refusal, worded from the
//! ledger, and a ledger that passes is the import's own record and goes nowhere.
//!
//! The identity rows are the second deliverable. §4 of the spec makes every id a
//! lookup against the site's existing map, so a reader has to be able to see how
//! many records found an entity, how many are new, and which of the site's
//! entities no record of this dump claimed.

use crate::format::coverage::Reading;
use crate::format::params::UsedParam;
use serde::Serialize;
use std::collections::BTreeMap;

/// How a site's import came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Every field family the export carries has a disposition, so nothing the
    /// files say can have gone nowhere. This is the target; the round-trip laws
    /// are what prove the rest.
    Lossless,
    /// A field family with no disposition, or a ledger nothing has settled yet.
    /// The ingest is refused here.
    Unaccounted,
}

/// One field family that crossed, and what it became.
#[derive(Debug, Clone, Serialize)]
pub struct Carried {
    pub construct: String,
    pub becomes: String,
    pub count: usize,
}

/// One field family the protocol still cannot state, after the vocabulary's
/// components exist.
#[derive(Debug, Clone, Serialize)]
pub struct Unrepresentable {
    pub path: String,
    pub reading: Reading,
    pub count: usize,
    pub why: String,
}

/// Where the identities came from, and what the existing map holds that this dump
/// does not mention.
#[derive(Debug, Clone, Default, Serialize)]
pub struct IdentityAccount {
    /// How many entities the site's existing map held when the import ran.
    pub existing_entities: usize,
    /// Records that found exactly one existing entity under their lookup key and
    /// kept its identity.
    pub looked_up: usize,
    /// Records nothing matched, which took a fresh identity from the host.
    pub fresh: usize,
    /// The site's entities no record of this dump claimed. An ingest deletes the
    /// ones that are this adapter's, so a reader should see this list first.
    pub unclaimed_entities: usize,
    /// The first few of them, named. The whole list can be a site's worth of
    /// entities, and a ledger carries counts and instances rather than
    /// production data (spec §6).
    pub in_the_site_not_in_this_dump: Vec<String>,
    /// How many legacy keys one record uses that name nothing in this dump. Never
    /// corrected: the legacy string rides verbatim and the export writes it back.
    pub dangling_source_references: usize,
    /// The first few of those, quoted, so a reader can go and look.
    pub dangling_instances: Vec<String>,
}

/// How many instances of a list a ledger carries. Enough to go and look, far
/// short of a copy of the site.
pub const INSTANCES: usize = 3;

/// One site's whole account of one import.
#[derive(Debug, Clone, Serialize)]
pub struct Ledger {
    pub status: Status,
    /// Files the dump carried, and what each one's records became.
    pub files: BTreeMap<String, String>,
    pub identity: IdentityAccount,
    pub carried: Vec<Carried>,
    /// Facts carried under a `runtime` sub-object rather than dropped, so a later
    /// policy can drop them deliberately (spec §4).
    pub runtime_carried: Vec<Carried>,
    pub unrepresentable: Vec<Unrepresentable>,
    pub invented_parameters: Vec<UsedParam>,
    /// Field families the dump carries that no disposition table covers. Must be
    /// empty: a path nobody has disposed of is a path that could be lost
    /// unnoticed.
    pub uncovered: Vec<String>,
    pub notes: Vec<String>,
}

impl Default for Ledger {
    fn default() -> Ledger {
        Ledger::new()
    }
}

impl Ledger {
    /// An empty account, which has accounted for nothing yet and says so: it
    /// settles to a passing status only once [`Ledger::settle`] has read what
    /// the import put in it.
    pub fn new() -> Ledger {
        Ledger {
            status: Status::Unaccounted,
            files: BTreeMap::new(),
            identity: IdentityAccount::default(),
            carried: Vec::new(),
            runtime_carried: Vec::new(),
            unrepresentable: Vec::new(),
            invented_parameters: Vec::new(),
            uncovered: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn carry(&mut self, construct: impl Into<String>, becomes: impl Into<String>, count: usize) {
        if count == 0 {
            return;
        }
        self.carried.push(Carried { construct: construct.into(), becomes: becomes.into(), count });
    }

    pub fn carry_runtime(&mut self, construct: impl Into<String>, becomes: impl Into<String>, count: usize) {
        if count == 0 {
            return;
        }
        self.runtime_carried.push(Carried { construct: construct.into(), becomes: becomes.into(), count });
    }

    /// Record a fact the core protocol still cannot state, after the vocabulary's
    /// components exist. A count of zero is not recorded: a site with none of a
    /// thing has no question to raise about it, and a ledger of zeros buries the
    /// rows that are about this site.
    pub fn cannot_state(&mut self, path: impl Into<String>, reading: Reading, count: usize, why: impl Into<String>) {
        if count == 0 {
            return;
        }
        self.unrepresentable.push(Unrepresentable { path: path.into(), reading, count, why: why.into() });
    }

    /// The status the ledger's own contents imply, worked out once every count is
    /// in, so a site cannot be called lossless while carrying a path nobody has
    /// disposed of.
    pub fn settle(&mut self) {
        self.status = if self.uncovered.is_empty() { Status::Lossless } else { Status::Unaccounted };
    }

    /// Whether the import may go ahead: nothing the files carry is unaccounted
    /// for. Read off the settled status rather than assembled again at the call
    /// site, so one predicate decides it.
    pub fn passes(&self) -> bool {
        matches!(self.status, Status::Lossless)
    }

    /// One line saying how the import came out.
    pub fn headline(&self) -> String {
        format!(
            "{:?} — {} ids looked up, {} fresh, {} uncovered paths",
            self.status,
            self.identity.looked_up,
            self.identity.fresh,
            self.uncovered.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Ledger, Status};

    /// A ledger nothing has settled has accounted for nothing, so it does not
    /// pass: an account that started out passing would pass for any caller that
    /// forgot to settle it.
    #[test]
    fn an_unsettled_ledger_does_not_pass() {
        let ledger = Ledger::new();
        assert_eq!(ledger.status, Status::Unaccounted);
        assert!(!ledger.passes());
    }

    #[test]
    fn a_settled_import_with_every_path_covered_is_lossless() {
        let mut ledger = Ledger::new();
        ledger.settle();
        assert_eq!(ledger.status, Status::Lossless);
        assert!(ledger.passes());
    }

    /// An uncovered key path is a field family nobody has disposed of, and it
    /// fails the import whatever else the ledger says.
    #[test]
    fn an_uncovered_key_path_does_not_pass() {
        let mut uncovered = Ledger::new();
        uncovered.uncovered.push("map.json:[].new_thing".to_string());
        uncovered.settle();
        assert_eq!(uncovered.status, Status::Unaccounted);
        assert!(!uncovered.passes(), "an uncovered key path is a field family nobody has disposed of");
        assert!(uncovered.headline().contains("1 uncovered paths"), "{}", uncovered.headline());
    }
}
