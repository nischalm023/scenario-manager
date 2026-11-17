//! Within `wps-client`, `export` is a site's flagged export as a run reads it (rulings D40 and D185; spec 04 sections
//! 3.3 and 3.12): the whole site of a cut as world-proto-server generated it, with every entity it references, asked
//! for in one request (`nested`, `derived` and `referenced` all set, each scope of the cut's unit at its changeset),
//! held to the cut, every reference it makes resolved within it and every pin held to the changeset the file holds its
//! scope at before the pins are stripped, and written with every object's keys sorted so the API and a runner hash the
//! same file alike (ruling D77). Until complete annotation traversal lands, WPS follows only the references its index
//! holds, so this check is what refuses a reference out of the file, or one pinned at a version the file does not hold.

use std::collections::BTreeMap;

use bytes::Bytes;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;
use sha2::{Digest, Sha256};
use studio_protocol::{DeclaredId, EntityId, EntityRef, Kinship, Pin, RefPin, ScopePath, Seq};

use crate::compute::{self, Compute};
use crate::error::{Problem, ProblemKind, WpsError, spoken};
use crate::kinship::ScopeTree;
use crate::schema::{RegistrySnapshot, SchemaCache};
use crate::snapshot::{CutExport, Walked};
use crate::value::{Exactness, UnitCut};
use crate::walker::{Annotations, RefForm, components_read, own_list_digest, walk};
use crate::wire::ExportFlags;
use crate::wps::Wps;

/// The most scopes one flagged export holds entities from, those it requests and those its references bring entities
/// from together, as WPS bounds it (`wp:server/server/src/storage/flagged_export/mod.rs`).
pub const CLOSURE_LIMIT: usize = 256;

fn entries(file: &Value) -> &[Value] {
    file.get("entities").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// Every component whose annotations a walk of the file's entities reads.
fn component_names(file: &Value) -> FxHashSet<String> {
    let mut names = FxHashSet::default();
    for entry in entries(file) {
        let Some(components) = entry.get("components").and_then(Value::as_object) else { continue };
        for (name, payload) in components {
            names.extend(components_read(name, payload));
        }
    }
    names
}

/// The flagged export of `cut`, all three flags set: asked of WPS, closed and written as a run reads it. `tree` is the
/// scope listing whose boundaries tell the held scopes' units apart, and whose heads say which held scope was read
/// below its head, which makes the export inexact.
pub async fn flagged<S: Wps>(
    wps: &S,
    pool: &dyn Compute,
    schemas: &SchemaCache,
    tree: &ScopeTree,
    cut: &UnitCut,
) -> Result<CutExport, WpsError> {
    let raw = wps.export_flagged(&cut.unit, ExportFlags::ALL, &cut.seqs).await?;
    let read = compute::run(pool, move || {
        let file: Value = serde_json::from_slice(&raw).map_err(|fault| fault.to_string())?;
        let names = component_names(&file);
        Ok::<_, String>((file, names))
    })
    .await?;
    let (file, names) = read.map_err(|fault| WpsError::Invalid {
        text: format!("the flagged export of {} is not JSON: {fault}", cut.unit),
    })?;
    for name in &names {
        schemas.ensure(wps, name).await?;
    }
    let (registry, asked, units) = (schemas.snapshot(), cut.clone(), tree.clone());
    let Closed { bytes, scopes, holders } =
        compute::run(pool, move || close(&asked, file, &names, &registry, &units)).await??;
    let exact = scopes.iter().fold(cut.exact.clone(), |exact, (scope, seq)| {
        match tree.head(scope).is_some_and(|head| head > *seq) {
            true => exact.and(Exactness::below_head(scope, *seq)),
            false => exact,
        }
    });
    let sha256 = hex::encode(Sha256::digest(&bytes));
    Ok(CutExport {
        bytes,
        sha256,
        scopes,
        tag: None,
        exact,
        walked: Walked::AnnotationsAndList { list: own_list_digest() },
        holders,
    })
}

/// `file` as a run reads it: compact JSON, every object's keys in sorted order at every depth. serde_json writes a map
/// in insertion order once any crate linked into the binary turns on its `preserve_order` feature (rig-core does, in
/// the API), and sorted otherwise (a runner), so the export is written sorted everywhere and its SHA-256, a plan's
/// `world_sha256`, matches across them.
fn written(file: Value) -> Result<Bytes, WpsError> {
    let bytes = serde_json::to_vec(&sorted(file))
        .map_err(|fault| WpsError::Invalid { text: format!("the flagged export does not serialize: {fault}") })?;
    Ok(Bytes::from(bytes))
}

/// `value` with every object's keys in sorted order, comparing bytes.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut fields: Vec<(String, Value)> = fields.into_iter().collect();
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(fields.into_iter().map(|(key, value)| (key, sorted(value))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// What `close` makes of a file: its bytes as a run reads them, every scope it holds entities from at the changeset
/// it was read at, and the scope that holds each declared entity it lists.
#[derive(Debug)]
struct Closed {
    bytes: Bytes,
    // ordered: it becomes the export's `scopes`, which a runner's manifest lists in path order.
    scopes: BTreeMap<ScopePath, Seq>,
    holders: FxHashMap<DeclaredId, ScopePath>,
}

/// Hold the file WPS wrote for `cut` to it, and write it as a run reads it: the file is the export of the cut's unit
/// with all three flags set, and holds each scope of the cut whole at the cut's changeset and every other scope only
/// in part; it lists each identity once, in a scope it holds; every reference in it names one of its own entities, a
/// pinned one at the changeset the file holds its scope at and one at head within the referrer's unit, as `units`
/// tells the scopes' units apart; and every component it carries, of `names`, is one the registry defines, since a
/// component it does not define has no annotations, and reading it as holding no reference would vouch for a closure
/// nobody checked. Only then are its pins stripped.
fn close(
    cut: &UnitCut,
    mut file: Value,
    names: &FxHashSet<String>,
    registry: &RegistrySnapshot,
    units: &ScopeTree,
) -> Result<Closed, WpsError> {
    let invalid = |what: String| WpsError::Invalid { text: format!("the flagged export of {}: {what}", cut.unit) };
    let named = file.get("scope").and_then(Value::as_str);
    if named != Some(cut.unit.as_str()) {
        return Err(invalid(format!("it names {named:?} as the scope it exports")));
    }
    let unset = unset_flags(&file).map_err(invalid)?;
    if !unset.is_empty() {
        let asked = ExportFlags::ALL.named().map(|(flag, _)| format!("{flag}=1")).join("&");
        return Err(invalid(format!("it does not state {} as true, which `?{asked}` asks for", spoken(&unset))));
    }
    let held = held_scopes(&file).map_err(invalid)?;
    for (scope, seq) in &cut.seqs {
        match held.get(scope) {
            Some((read, true)) if read == seq => {}
            Some((read, true)) => return Err(invalid(format!("it holds {scope} at changeset {read}, not {seq}"))),
            Some((_, false)) => {
                return Err(invalid(format!("it holds {scope} in part, and the cut asks for it whole")));
            }
            None => return Err(invalid(format!("it does not hold {scope}"))),
        }
    }
    if let Some((stray, _)) = held.iter().find(|(scope, (_, whole))| *whole && !cut.seqs.contains_key(*scope)) {
        return Err(invalid(format!("it holds {stray} whole, and {stray} is no scope of the cut")));
    }
    let scopes: BTreeMap<ScopePath, Seq> = held.into_iter().map(|(scope, (seq, _))| (scope, seq)).collect();
    let mut problems = Vec::new();
    for name in registry.missing(names.iter().map(String::as_str)) {
        let text = format!("component `{name}` is carried and the registry defines none, so its references go unread");
        problems.push(Problem::new(ProblemKind::Unregistered, text));
    }
    let Some(listed) = file.get_mut("entities").and_then(Value::as_array_mut) else {
        return Err(invalid("it lists no `entities`".to_owned()));
    };
    // Each identity as a reference spells it, and the scope that holds it.
    let mut scope_of: FxHashMap<String, ScopePath> = FxHashMap::default();
    let mut declared = FxHashMap::default();
    for entry in listed.iter() {
        let (id, scope) = identity(entry).map_err(invalid)?;
        if !scopes.contains_key(&scope) {
            return Err(invalid(format!("it lists {id} in {scope}, which its `scopes` does not name")));
        }
        if let Some(first) = scope_of.insert(id.to_string(), scope.clone()) {
            let problem = Problem::new(ProblemKind::HeldTwice, format!("{id} is held by {first} and by {scope}"));
            problems.push(problem.in_scope(&scope).on(id));
            continue;
        }
        if EntityId::from(id.declared()) == id {
            declared.insert(id.declared(), scope);
        }
    }
    for entry in listed.iter() {
        problems.extend(unclosed(entry, &scope_of, &scopes, registry, units));
    }
    if !problems.is_empty() {
        return Err(WpsError::ExportNotClosed { problems });
    }
    for entry in listed.iter_mut() {
        strip_pins(entry, registry);
    }
    Ok(Closed { bytes: written(file)?, scopes, holders: declared })
}

/// The flags, of the three a run's export asks for, that the file does not state as true; or why it states one as
/// neither `true` nor `false`.
fn unset_flags(file: &Value) -> Result<Vec<String>, String> {
    let mut unset = Vec::new();
    for (flag, _) in ExportFlags::ALL.named() {
        match file.get(flag) {
            Some(Value::Bool(true)) => {}
            None | Some(Value::Bool(false)) => unset.push(format!("`{flag}`")),
            Some(other) => return Err(format!("it states `{flag}` as {other}, which is neither `true` nor `false`")),
        }
    }
    Ok(unset)
}

/// The file's `scopes`: each scope it holds entities from, with the changeset it was read at and whether the file
/// holds it whole.
fn held_scopes(file: &Value) -> Result<BTreeMap<ScopePath, (Seq, bool)>, String> {
    let listed = file.get("scopes").and_then(Value::as_array).ok_or("it lists no `scopes`")?;
    // ordered: they become the export's `scopes`, which a runner's manifest lists in path order.
    let mut held = BTreeMap::new();
    for one in listed {
        let scope = one.get("scope").and_then(Value::as_str).and_then(|scope| scope.parse::<ScopePath>().ok());
        let seq = one.get("changeset").and_then(Value::as_u64).map(Seq::new);
        let whole = one.get("whole").and_then(Value::as_bool);
        let (Some(scope), Some(seq), Some(whole)) = (scope, seq, whole) else {
            return Err(format!("{one} in `scopes` is not a scope with a changeset and whether it is whole"));
        };
        if let Some((earlier, _)) = held.insert(scope.clone(), (seq, whole)) {
            return Err(format!("it lists {scope} twice, at changesets {earlier} and {seq}"));
        }
    }
    Ok(held)
}

/// An entity's identity, which must read as one, and the scope that holds it.
fn identity(entry: &Value) -> Result<(EntityId, ScopePath), String> {
    let id = entry.get("id").and_then(Value::as_str).ok_or_else(|| format!("an entity without an `id`: {entry}"))?;
    let id: EntityId =
        id.parse().map_err(|fault| format!("{id} is listed as an entity and is no identity: {fault}"))?;
    let scope = entry.get("scope").and_then(Value::as_str).and_then(|scope| scope.parse().ok());
    let scope = scope.ok_or_else(|| format!("{id} is listed with no scope that holds it"))?;
    Ok((id, scope))
}

/// Write bare every pinned reference the walker reads in `entry`'s components.
pub(crate) fn strip_pins(entry: &mut Value, annotations: &dyn Annotations) {
    let Some(components) = entry.get_mut("components").and_then(Value::as_object_mut) else { return };
    for (name, payload) in components.iter_mut() {
        for occurrence in walk(name, payload, annotations) {
            let RefForm::Text(text) = &occurrence.form else { continue };
            if let Some((bare, _)) = text.split_once('@')
                && let Some(slot) = payload.pointer_mut(&occurrence.path)
            {
                *slot = Value::String(bare.to_owned());
            }
        }
    }
}

/// The references of one entity the file does not resolve: one naming no entity the file holds; one whose pin does
/// not read, or names a tag or a branch, which name no changeset to hold the file to; one pinned at another
/// changeset than the file holds its target's scope at, which stripping the pin would move to the version held; and
/// one at head naming another unit's entity, bare, `@head` or a derivation address in its two parts, which carry no
/// pin: the pin rule refuses it, WPS only where the annotations reach (`head_across_units`, `two_part_across_units`),
/// and it would read whatever changeset another pin had the file hold its scope at.
fn unclosed(
    entry: &Value,
    scope_of: &FxHashMap<String, ScopePath>,
    scopes: &BTreeMap<ScopePath, Seq>,
    registry: &RegistrySnapshot,
    units: &ScopeTree,
) -> Vec<Problem> {
    let mut problems = Vec::new();
    let Ok((referrer, scope)) = identity(entry) else { return problems };
    let refuse = |kind: ProblemKind, component: &str, path: &str, why: String| {
        let problem = Problem::new(kind, format!("{referrer} in {scope}: {why}"));
        problem.in_scope(&scope).at_field(component, path).on(referrer.clone())
    };
    let Some(components) = entry.get("components").and_then(Value::as_object) else { return problems };
    for (name, payload) in components {
        for occurrence in walk(name, payload, registry) {
            let here = |kind: ProblemKind, why: String| refuse(kind, name, &occurrence.path, why);
            // `text` is the reference written as one string, none for a derivation address in its two parts.
            let (named, pin, text) = match &occurrence.form {
                RefForm::Text(text) => match pinned(text) {
                    Ok((named, pin)) => (named.to_owned(), pin, Some(text)),
                    Err((kind, why)) => {
                        problems.push(here(kind, why));
                        continue;
                    }
                },
                RefForm::TwoPart { generator, key } => (format!("{generator}/{key}"), None, None),
                RefForm::Local { known: true, .. } => continue,
                RefForm::Local { handle, .. } => {
                    let why = format!("`{handle}` names no sibling inside its template");
                    problems.push(here(ProblemKind::Dangling, why));
                    continue;
                }
                RefForm::Malformed(value) => {
                    problems.push(here(ProblemKind::BadReference, format!("{value} is not a reference")));
                    continue;
                }
            };
            let Some(target) = scope_of.get(&named) else {
                let why = format!("it names {named}, which the export does not hold");
                problems.push(here(ProblemKind::Dangling, why));
                continue;
            };
            let spell = || match text {
                Some(text) => format!("`{text}`"),
                None => format!("`{named}`, written in its two parts,"),
            };
            match (pin, scopes.get(target)) {
                (Some(pin), Some(at)) if pin != *at => {
                    let spelled = spell();
                    let why = format!(
                        "{spelled} pins {target} at changeset {pin}, and the export holds {target} at changeset {at}"
                    );
                    problems.push(here(ProblemKind::PinnedTwice, why));
                }
                (None, _) if units.kinship(&scope, target) == Kinship::Lateral => {
                    let spelled = spell();
                    let why = format!(
                        "{spelled} names an entity of {target} at its head, and {target} is no scope of {scope}'s \
                         unit, where the pin rule allows that"
                    );
                    problems.push(here(ProblemKind::HeadAcrossUnits, why));
                }
                _ => {}
            }
        }
    }
    problems
}

/// The identity a reference written as one string names, and the changeset its pin names: none where it is bare or
/// pinned `@head`, which the pin rule allows only within the referrer's unit, whose scopes the file holds at one
/// changeset each. Or why the file cannot hold it to its pin: it does not read, or it names a tag, which no scope holds
/// before world-proto-server builds tags, or a branch, which moves.
fn pinned(text: &str) -> Result<(&str, Option<Seq>), (ProblemKind, String)> {
    let Some((named, _)) = text.split_once('@') else { return Ok((text, None)) };
    let reference = text.parse::<EntityRef>().map_err(|fault| (ProblemKind::BadReference, fault.to_string()))?;
    match reference.pin() {
        Some(RefPin::Pin(Pin::Changeset(seq))) => Ok((named, Some(*seq))),
        Some(RefPin::Head) | None => Ok((named, None)),
        Some(RefPin::Pin(Pin::Tag(tag))) => Err((
            ProblemKind::TagNotHeld,
            format!("`{text}` pins tag {tag}, and no scope holds a tag until world-proto-server builds tags"),
        )),
        Some(RefPin::Pin(Pin::Branch(branch))) => Err((
            ProblemKind::BadReference,
            format!("`{text}` pins branch {branch}, which names no one changeset of its scope"),
        )),
    }
}

#[cfg(test)]
mod tests;
