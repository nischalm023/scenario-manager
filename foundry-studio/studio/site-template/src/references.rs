//! Within `site-template`, `references` finds every reference position in an entity's components (spec 05 section
//! 4.3, step 6.10): each path the registry annotates (`/members/*`, `/from/node`), a stitch's explicit pairs, which
//! no annotation reaches (`/pairs/*/*`), and the component maps nested in a `template` and in a tiling's `derives`,
//! whose references belong to the entities they become. Which fields hold references is each component's schema to
//! say and is never guessed from a value's shape (the reader's `references` module makes the same choice).

use serde_json::{Map, Value};

use crate::read::RegistrySnapshot;

/// What a visitor answers for one value at a reference position.
pub(crate) enum Rewrite {
    Keep,
    Replace(Value),
    /// Only at a list position: the element gives way to these, none to drop it.
    Splice(Vec<Value>),
}

/// One value at a reference position: the component, the annotated path, and whether it is a list's element.
pub(crate) struct Position<'v> {
    pub component: &'v str,
    pub path: &'v str,
    pub in_list: bool,
    pub value: &'v Value,
}

/// Paths the registry's annotations cannot hold, per component: a pair is a list inside a list, and the server's
/// extractor follows one trailing `/*` (world-proto `server/scripts/seed-demo.sh`, `refs_for`).
fn unannotated(component: &str) -> &'static [&'static str] {
    match component {
        "stitch" => &["/pairs/*/*"],
        _ => &[],
    }
}

/// Offer every value at a reference position of `components` to `visit`, and apply its answers.
pub(crate) fn rewrite(
    components: &mut Map<String, Value>,
    registry: &RegistrySnapshot,
    visit: &mut dyn FnMut(Position<'_>) -> Rewrite,
) {
    for (name, value) in components.iter_mut() {
        if name.starts_with('$') {
            continue;
        }
        let mut paths: Vec<&str> = registry.refs(name).iter().map(|annotation| annotation.path.as_str()).collect();
        paths.extend(unannotated(name));
        paths.sort_unstable();
        paths.dedup();
        for path in paths {
            let segments: Vec<&str> = path.split('/').skip(1).collect();
            // A pair's two ends are one statement, so neither is a list element to drop or splice.
            let list = segments.last() == Some(&"*") && !unannotated(name).contains(&path);
            walk(value, &segments, name, path, list, visit);
        }
        match name.as_str() {
            "template" => {
                let entities = value.get_mut("entities").and_then(Value::as_array_mut);
                for internal in entities.into_iter().flatten() {
                    if let Some(nested) = internal.get_mut("components").and_then(Value::as_object_mut) {
                        rewrite(nested, registry, visit);
                    }
                }
            }
            "tiling" => {
                if let Some(derives) = value.get_mut("derives").and_then(Value::as_object_mut) {
                    rewrite(derives, registry, visit);
                }
            }
            _ => {}
        }
    }
}

fn walk(
    value: &mut Value,
    segments: &[&str],
    component: &str,
    path: &str,
    list: bool,
    visit: &mut dyn FnMut(Position<'_>) -> Rewrite,
) {
    match segments {
        [] => {
            if let Rewrite::Replace(replacement) = visit(Position { component, path, in_list: false, value }) {
                *value = replacement;
            }
        }
        ["*"] if list => {
            let Some(items) = value.as_array_mut() else { return };
            let mut out = Vec::with_capacity(items.len());
            for item in items.drain(..) {
                match visit(Position { component, path, in_list: true, value: &item }) {
                    Rewrite::Keep => out.push(item),
                    Rewrite::Replace(replacement) => out.push(replacement),
                    Rewrite::Splice(replacements) => out.extend(replacements),
                }
            }
            *items = out;
        }
        ["*", rest @ ..] => {
            for item in value.as_array_mut().into_iter().flatten() {
                walk(item, rest, component, path, list, visit);
            }
        }
        [field, rest @ ..] => {
            if let Some(inner) = value.get_mut(*field) {
                walk(inner, rest, component, path, list, visit);
            }
        }
    }
}
