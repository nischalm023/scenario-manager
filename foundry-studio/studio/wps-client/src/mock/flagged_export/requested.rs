//! Within the mock's flagged export, `requested` is WHAT A REQUEST ASKS FOR, AS WPS SETTLES IT: the scope it names and,
//! with `nested`, the scopes nested in it, which the file holds whole; and the changeset it reads each scope the
//! export reads at, as `at` names it or at its head: the requested scopes, and with `referenced` every scope of their
//! units, where a reference at head from them can land. Or WPS's refusal of an `at` those scopes do not fit, or of
//! more requested scopes than one export holds, before anything is read.

use std::collections::{BTreeMap, BTreeSet};

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::json;
use studio_protocol::{ScopePath, Seq};

use super::{CLOSURE_LIMIT, closure_too_large};
use crate::error::{WpsError, spoken};
use crate::kinship::ancestry;
use crate::mock::{Inner, missing, refuse};
use crate::refusal::RouteKind;
use crate::wire::ExportFlags;

/// What one request asks for: its flags, the scopes it holds whole, and the changeset it reads each scope the export
/// reads at.
pub(super) struct Request {
    pub(super) flags: ExportFlags,
    // ordered: the file lists its scopes in path order.
    pub(super) whole: BTreeSet<ScopePath>,
    pub(super) at: FxHashMap<ScopePath, Seq>,
}

impl Inner {
    /// The scopes the flagged export of `scope` holds whole, `scope` and with `nested` those nested in it, and the
    /// changeset it reads each scope the export reads at: the one `named` gives, or each at its head where `named`
    /// gives none; or WPS's refusal of a request those scopes cannot answer as asked.
    pub(super) fn request(
        &self,
        scope: &ScopePath,
        flags: ExportFlags,
        named: &BTreeMap<ScopePath, Seq>,
    ) -> Result<Request, WpsError> {
        if !self.scopes.contains_key(scope) {
            return Err(missing(RouteKind::Export, scope));
        }
        let mut whole = BTreeSet::from([scope.clone()]);
        if flags.nested {
            whole.extend(self.nested_in(scope));
        }
        if whole.len() > CLOSURE_LIMIT {
            return Err(closure_too_large(scope));
        }
        let units: FxHashSet<&ScopePath> = whole.iter().map(|path| &self.scopes[path].unit).collect();
        let read =
            |path: &ScopePath| whole.contains(path) || flags.referenced && units.contains(&self.scopes[path].unit);
        // ordered: an `at` left out is named in path order, as WPS names it.
        let reads: BTreeMap<&ScopePath, Seq> =
            self.scopes.iter().filter(|(path, _)| read(path)).map(|(path, held)| (path, held.head())).collect();
        if named.is_empty() {
            let at = reads.into_iter().map(|(path, head)| (path.clone(), head)).collect();
            return Ok(Request { flags, whole, at });
        }
        for (stated, seq) in named {
            let Some(head) = reads.get(stated) else {
                let sentence = format!(
                    "`at` names {stated}, which the export of {scope} does not read — name each scope it reads, and \
                     no other: the scopes it requests, and with `referenced` every scope a reference at head from them \
                     can reach"
                );
                return Err(refuse(RouteKind::Export, 409, "at_out_of_reach", json!({ "scope": stated }), sentence));
            };
            if seq > head {
                let sentence = format!("`at` names changeset {seq} of {stated}, whose head is {head}");
                let details = json!({ "scope": stated, "at": seq, "head": head });
                return Err(refuse(RouteKind::Export, 409, "at_beyond_head", details, sentence));
            }
        }
        let left: Vec<&ScopePath> = reads.keys().copied().filter(|path| !named.contains_key(*path)).collect();
        if !left.is_empty() {
            let sentence = format!(
                "`at` leaves out {}, which the export reads — name every scope it reads, or none to read each at \
                 its head",
                spoken(&left)
            );
            return Err(refuse(RouteKind::Export, 409, "at_left_out", json!({ "scopes": left }), sentence));
        }
        let at = named.iter().map(|(path, seq)| (path.clone(), *seq)).collect();
        Ok(Request { flags, whole, at })
    }

    /// The scopes nested in `scope`: every scope beneath it that no boundary stands over below it, so a boundary and
    /// everything beneath it are left out.
    fn nested_in(&self, scope: &ScopePath) -> Vec<ScopePath> {
        let open = |path: &ScopePath| {
            let mut between = ancestry(path).into_iter().take_while(|above| above != scope);
            between.all(|above| self.scopes.get(&above).is_none_or(|held| !held.boundary))
        };
        self.scopes.keys().filter(|path| *path != scope && scope.holds(path) && open(path)).cloned().collect()
    }
}
