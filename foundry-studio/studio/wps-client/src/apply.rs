//! Within `wps-client`, `apply` is spec 04 section 3.4's write: a draft goes to world-proto-server as ONE changeset
//! initiated on the draft's scope (ruling D72, scopes phase 3 at 092d695), each operation naming the scope it writes,
//! with a base for every other scope it writes or only reads, the scopes it creates, each scope's prologue and the
//! generator set it expects to run, all checked and applied atomically by WPS (the probe refuses a server without
//! it at start). Every write names its operation identifier, or is refused before anything is sent. One draft makes
//! one body (`changeset_of`), which its caller keeps with the approval: an answer lost once is sent again under the
//! same identifier, and WPS answers it with the first sending's whole answer, replayed; after a second loss, or a
//! restart, `resend` sends the kept body again, as it was and reading nothing first, once it is held to the draft it
//! settles. Only WPS's own refusal says nothing was written (`NotLanded`, LEAD-DECISIONS entry 54).

use rustc_hash::FxHashSet;
use semver::Version;
use studio_protocol::{ScopePath, Seq};

use crate::client::Client;
use crate::draft::{Applied, ChangesetDraft};
use crate::error::{ProblemKind, WpsError};
use crate::value::{GenerationRan, GeneratorSet};
use crate::walker::{components_read, respell_heads};
use crate::wire::{Respelled, WireApplied, WireChangeset, WireGeneration, WireOp, WireRanGenerator};
use crate::wps::{PostFailure, Wps};

/// The longest operation identifier WPS takes, in characters.
pub const OPERATION_CHARS: usize = 128;

/// The generator set as a changeset states it, in name order.
pub fn stated_generators(set: &GeneratorSet) -> Vec<WireRanGenerator> {
    set.0
        .iter()
        .map(|(generator, (version, digest))| WireRanGenerator {
            generator: generator.clone(),
            version: version.to_string(),
            digest: digest.clone(),
        })
        .collect()
}

/// The one changeset `draft` is sent as, and the body its caller keeps: the initiating scope's base and prologue in
/// `base` and `prologue`, every other scope's in `bases` and `prologues`, the created scopes in path order, and the
/// draft's operation identifier, without which nothing is built. One draft always makes one body, so a resend is a
/// replay.
pub fn changeset_of(draft: &ChangesetDraft) -> Result<WireChangeset, WpsError> {
    check_operation(draft)?;
    Ok(body_of(draft))
}

/// The body `changeset_of` makes, its identifier unchecked, as `check` measures it against WPS's limit.
pub(crate) fn body_of(draft: &ChangesetDraft) -> WireChangeset {
    let initiating = &draft.initiated_on;
    let mut prologues = draft.prologues.clone();
    let prologue = prologues.remove(initiating);
    let mut create = draft.creates.clone();
    create.sort_by(|a, b| a.path.cmp(&b.path));
    WireChangeset {
        scope: initiating.clone(),
        base: draft.bases.get(initiating).copied().unwrap_or(Seq::new(0)),
        author: draft.author.as_str().to_owned(),
        message: draft.message.clone(),
        ops: draft
            .ops
            .iter()
            .map(|scoped| WireOp {
                op: scoped.op.clone(),
                scope: (scoped.scope != *initiating).then(|| scoped.scope.clone()),
            })
            .collect(),
        prologue,
        operation: Some(draft.operation.clone()),
        bases: draft
            .bases
            .iter()
            .filter(|(scope, _)| *scope != initiating)
            .map(|(scope, base)| (scope.clone(), *base))
            .collect(),
        create,
        prologues,
        generators: draft.expect_generators.as_ref().map(stated_generators),
    }
}

/// The draft's operation identifier held to WPS's 1 to 128 characters without U+0000 before anything is sent: every
/// write names one, because only WPS's replay of it settles an answer that was lost.
pub fn check_operation(draft: &ChangesetDraft) -> Result<(), WpsError> {
    let chars = draft.operation.chars().count();
    if chars == 0 || chars > OPERATION_CHARS || draft.operation.contains('\0') {
        let text = format!(
            "the operation identifier `{}` has {chars} characters, and every write names one of 1 to \
             {OPERATION_CHARS} without U+0000; nothing was sent",
            draft.operation
        );
        return Err(WpsError::Invalid { text });
    }
    Ok(())
}

pub(crate) fn generation_ran(wire: &WireGeneration) -> GenerationRan {
    let generators = wire
        .generators
        .iter()
        .filter_map(|ran| Some((ran.generator.clone(), (Version::parse(&ran.version).ok()?, ran.digest.clone()))))
        .collect();
    GenerationRan { digest: wire.digest.clone(), derived: wire.derived, generators: GeneratorSet(generators) }
}

/// Every scope whose change an unknown outcome leaves in doubt: those the draft writes and those it creates.
fn in_doubt(draft: &ChangesetDraft) -> Vec<ScopePath> {
    let mut scopes = draft.written();
    scopes.extend(draft.creates.iter().map(|new| new.path.clone()));
    scopes.sort();
    scopes.dedup();
    scopes
}

/// How sending a changeset ended short of landing. `Refused` is WPS's own refusal of the body, read from its refusal
/// object, which stored nothing under the body's operation identifier, so nothing was written. `Unknown` is anything
/// else (an answer lost twice, a 5xx, a 201 that did not read, a body this client would not send as kept): whether it
/// landed is not known, and only sending the same body again under the same identifier settles it.
#[derive(Debug, Clone, PartialEq)]
pub enum NotLanded {
    Refused(WpsError),
    Unknown(WpsError),
}

impl NotLanded {
    pub fn into_error(self) -> WpsError {
        match self {
            Self::Refused(error) | Self::Unknown(error) => error,
        }
    }
}

impl<S: Wps> Client<S> {
    pub(crate) async fn apply_draft(&self, draft: &ChangesetDraft) -> Result<Applied, WpsError> {
        let body = changeset_of(draft)?;
        self.send(draft, &body).await
    }

    /// `sent`, the body `draft` was first sent as and its caller kept, sent again as it was under its operation
    /// identifier, reading no schema first, once it is held to the draft: a body other than the one the draft makes
    /// is not sent, its outcome left unknown, because WPS's replay of it would answer for another draft.
    pub(crate) async fn resend_draft(
        &self,
        sent: &WireChangeset,
        draft: &ChangesetDraft,
    ) -> Result<Applied, NotLanded> {
        if changeset_of(draft).map_err(NotLanded::Unknown)? != *sent {
            let text = format!(
                "the body kept for operation `{}` is not the changeset its draft makes, so nothing was sent",
                draft.operation
            );
            return Err(NotLanded::Unknown(WpsError::Invalid { text }));
        }
        self.post(draft, sent).await
    }

    /// A first sending: the schemas of what `draft` writes read into the cache, for the respellings its answer owes,
    /// then `body` posted; a `schema_invalid` refusal naming a version the cache does not hold refreshes that
    /// component.
    pub(crate) async fn send(&self, draft: &ChangesetDraft, body: &WireChangeset) -> Result<Applied, WpsError> {
        for op in &draft.ops {
            for (name, payload) in op.op.components().into_iter().flatten() {
                for read in components_read(name, payload) {
                    self.schemas.ensure(&*self.wps, &read).await?;
                }
            }
        }
        match self.post(draft, body).await {
            Err(NotLanded::Refused(WpsError::Refused { problem })) => {
                if let (ProblemKind::SchemaInvalid, Some(component), Some(version)) =
                    (problem.kind, &problem.component, &problem.version)
                {
                    let _ = self.schemas.note_refusal(&*self.wps, component, version).await;
                }
                Err(WpsError::Refused { problem })
            }
            answered => answered.map_err(NotLanded::into_error),
        }
    }

    /// Post `body`, the changeset `draft` makes, as it is and reading nothing first, and post it again once on WPS's
    /// word that nothing was written, or on a lost answer; a second of either ends the write.
    pub(crate) async fn post(&self, draft: &ChangesetDraft, body: &WireChangeset) -> Result<Applied, NotLanded> {
        let (mut resent, mut resent_lost) = (false, false);
        loop {
            let failure = match self.wps.post_changeset(body).await {
                Ok(answer) => return Ok(self.applied(draft, body, answer)),
                Err(failure) => failure,
            };
            let unavailable = |last: String| NotLanded::Refused(WpsError::Unavailable { attempts: 2, last });
            match failure {
                PostFailure::EndedByPostgres { .. } if !resent => resent = true,
                PostFailure::EndedByPostgres { text } => return Err(unavailable(text)),
                // A draft stating its generator set is not resent: its evaluation is approved again (section 3.8).
                PostFailure::Answered(WpsError::RegistryMoved { .. })
                    if draft.expect_generators.is_none() && !resent =>
                {
                    resent = true;
                }
                PostFailure::Answered(WpsError::RegistryMoved { text, .. }) if draft.expect_generators.is_none() => {
                    return Err(unavailable(text));
                }
                PostFailure::Answered(refusal) => return Err(NotLanded::Refused(refusal)),
                // A lost answer, a 500 included, is unknown: the same body goes again under the same identifier, never
                // a new one, and WPS answers a first sending that landed with its replay.
                PostFailure::Lost { .. } if !resent_lost => resent_lost = true,
                PostFailure::Lost { .. } => {
                    return Err(NotLanded::Unknown(WpsError::OutcomeUnknown { scopes: in_doubt(draft) }));
                }
            }
        }
    }

    /// What the whole changeset answered: each written scope's changeset, generation and archived records, each
    /// created scope, and a warning for each scope the draft writes that the answer leaves out and for each
    /// respelling WPS reported other than the ones it owes.
    fn applied(&self, draft: &ChangesetDraft, sent: &WireChangeset, answer: WireApplied) -> Applied {
        let mut applied = Applied { created: answer.created.clone(), replayed: answer.replayed, ..Applied::default() };
        for part in answer.parts(sent) {
            applied.generation.insert(part.scope.clone(), part.generation.as_ref().map(generation_ran));
            applied.seqs.insert(part.scope, part.seq);
            applied.archived.extend(part.archived);
        }
        for scope in draft.written().into_iter().filter(|scope| !applied.seqs.contains_key(scope)) {
            applied
                .warnings
                .push(format!("world-proto-server's answer names no changeset of {scope}, which this draft writes"));
        }
        applied.warnings.extend(self.respelling_warnings(draft, &answer.respelled));
        applied.warnings.extend(answer.warnings);
        applied
    }

    /// The respellings WPS owes the draft: every `@head` at an annotated place of what it writes, stored bare (spec
    /// 04 section 3.7).
    fn expected_respellings(&self, draft: &ChangesetDraft) -> Vec<Respelled> {
        let registry = self.schemas.snapshot();
        let mut expected = Vec::new();
        for scoped in &draft.ops {
            if let Some(components) = scoped.op.components() {
                let mut written = components.clone();
                expected.extend(respell_heads(&scoped.op.entity().to_string(), &mut written, &*registry));
            }
        }
        expected
    }

    /// A warning, in a stable order, for each respelling WPS reported and does not owe, and each it owes and did not
    /// report; what landed stands either way.
    fn respelling_warnings(&self, draft: &ChangesetDraft, reported: &[Respelled]) -> Vec<String> {
        let expected: FxHashSet<Respelled> = self.expected_respellings(draft).into_iter().collect();
        let reported: FxHashSet<Respelled> = reported.iter().cloned().collect();
        let at = |one: &Respelled| format!("{}.{}{}", one.entity, one.component, one.path);
        let mut warnings: Vec<(String, String)> = reported
            .difference(&expected)
            .map(|one| {
                let text = format!(
                    "world-proto-server stored `{}` at {} as `{}`, a respelling Agent Studio did not expect",
                    one.written,
                    at(one),
                    one.stored
                );
                (at(one), text)
            })
            .chain(expected.difference(&reported).map(|one| {
                let text = format!(
                    "Agent Studio wrote `{}` at {} to be stored bare, and world-proto-server did not say it was",
                    one.written,
                    at(one)
                );
                (at(one), text)
            }))
            .collect();
        warnings.sort();
        warnings.into_iter().map(|(_, text)| text).collect()
    }
}
