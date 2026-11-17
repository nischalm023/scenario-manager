//! Within `wps-client`, `walker` finds every reference in a component payload (spec 04 section 3.7), for `check` and
//! for the export's closure check alike: the locations the registry's annotations name, as WPS reads them (a JSON
//! pointer, or one trailing `/*`, `wp:server/server/src/storage/references.rs:118-162`), and the locations they leave
//! out at 5e66d0c (`wp:server/scripts/seed-demo.sh:272-285`), from a list of its own that WPS never checks. The
//! annotated places alone are where WPS indexes and respells a reference, so they are walked apart too.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use studio_protocol::{EntityRef, RefPin};

use crate::wire::{RefAnnotation, Respelled};

/// The component locations the annotations miss at 5e66d0c: each pair of an explicit stitch and the frame of its
/// curve, the entities inside a template and the component map a tiling derives (each walked with its own
/// components' annotations), and a port's entries where one parses as an identity. The contract suite holds it to
/// every registered component; it is deleted once complete annotation traversal lands (section 3.12).
pub const OWN_LIST: &[(&str, &str)] = &[
    ("stitch", "/pairs/*/*"),
    ("stitch", "/curve/frame"),
    ("template", "/entities/*/components"),
    ("tiling", "/derives"),
    ("port", "/of/*"),
];

/// The digest an export's `walked` names for the list it walked with.
pub fn own_list_digest() -> String {
    let text: Vec<String> = OWN_LIST.iter().map(|(component, path)| format!("{component} {path}")).collect();
    hex::encode(Sha256::digest(text.join("\n").as_bytes()))
}

/// How a reference is written.
#[derive(Debug, Clone, PartialEq)]
pub enum RefForm {
    /// One string: an id or a derivation address, with an optional `@` pin.
    Text(String),
    /// A derivation address in its two parts, which carries no pin.
    TwoPart { generator: String, key: String },
    /// A template internal's handle; `known` when a sibling inside the same template declares it.
    Local { handle: String, known: bool },
    /// A value at a reference location that is none of the above.
    Malformed(Value),
}

/// One reference found: the component it sits in and its JSON pointer there.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub component: String,
    pub path: String,
    pub form: RefForm,
}

/// The annotations a component's latest schema carries; none for a component nobody registered.
pub trait Annotations {
    fn refs(&self, component: &str) -> &[RefAnnotation];
}

/// The components a value standing where a component map belongs holds: its entries but those whose key begins with
/// `$`, which never names a component. WPS refuses such a key beside an entity's own components as an unregistered
/// component (`load_schemas` in `wp:server/server/src/storage/apply.rs`), and keeps `$comment` beside a template
/// internal's components as a note; a placeholder standing for the whole map (`{"$param": "derives"}`) is a stamp's to
/// fill, and holds none.
fn components_in(map: Option<&Value>) -> impl Iterator<Item = (&String, &Value)> {
    map.and_then(Value::as_object).into_iter().flatten().filter(|(name, _)| !name.starts_with('$'))
}

/// The components whose annotations a walk of `component`'s `payload` reads: it, and those nested inside a template's
/// entities or a tiling's derived map; none for a `$` key, which names no component.
pub fn components_read(component: &str, payload: &Value) -> Vec<String> {
    if component.starts_with('$') {
        return Vec::new();
    }
    let mut names = vec![component.to_owned()];
    let nested = match component {
        "template" => payload
            .get("entities")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|entity| components_in(entity.get("components")))
            .collect::<Vec<_>>(),
        "tiling" => components_in(payload.get("derives")).collect(),
        _ => Vec::new(),
    };
    for (inner, value) in nested {
        names.extend(components_read(inner, value));
    }
    names
}

/// Every reference in `component`'s `payload`; none for a `$` key, which names no component.
pub fn walk(component: &str, payload: &Value, annotations: &dyn Annotations) -> Vec<Occurrence> {
    let mut found = Vec::new();
    if !component.starts_with('$') {
        walk_into(component, "", payload, annotations, None, &mut found);
    }
    found
}

fn walk_into(
    component: &str,
    prefix: &str,
    payload: &Value,
    annotations: &dyn Annotations,
    locals: Option<&[String]>,
    found: &mut Vec<Occurrence>,
) {
    let mut push = |path: String, value: &Value| {
        if let Some(form) = form_of(value, locals) {
            found.push(Occurrence { component: component.to_owned(), path: format!("{prefix}{path}"), form });
        }
    };
    for annotation in annotations.refs(component) {
        for (path, value) in pointed(payload, &annotation.path) {
            push(path, value);
        }
    }
    for (_, location) in OWN_LIST.iter().filter(|(owner, _)| *owner == component) {
        match *location {
            "/pairs/*/*" => {
                for (path, pair) in pointed(payload, "/pairs/*") {
                    for (index, end) in pair.as_array().into_iter().flatten().enumerate() {
                        push(format!("{path}/{index}"), end);
                    }
                }
            }
            "/of/*" => {
                for (path, entry) in pointed(payload, location) {
                    match entry {
                        Value::String(text) if opens_with_uuid(text) => push(path, entry),
                        Value::Object(fields) => {
                            for (key, value) in fields {
                                if value.as_str().is_some_and(opens_with_uuid) {
                                    push(format!("{path}/{key}"), value);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "/entities/*/components" | "/derives" => {}
            plain => {
                for (path, value) in pointed(payload, plain) {
                    push(path, value);
                }
            }
        }
    }
    drop(push);
    match component {
        "template" => {
            let entities = payload.get("entities").and_then(Value::as_array).cloned().unwrap_or_default();
            let handles: Vec<String> =
                entities.iter().filter_map(|entity| entity.get("local")?.as_str().map(str::to_owned)).collect();
            for (index, entity) in entities.iter().enumerate() {
                for (inner, value) in components_in(entity.get("components")) {
                    let at = format!("{prefix}/entities/{index}/components/{inner}");
                    let mut nested = Vec::new();
                    walk_into(inner, &at, value, annotations, Some(&handles), &mut nested);
                    found.extend(
                        nested
                            .into_iter()
                            .map(|occurrence| Occurrence { component: component.to_owned(), ..occurrence }),
                    );
                }
            }
        }
        "tiling" => {
            for (inner, value) in components_in(payload.get("derives")) {
                let at = format!("{prefix}/derives/{inner}");
                let mut nested = Vec::new();
                walk_into(inner, &at, value, annotations, locals, &mut nested);
                found.extend(
                    nested.into_iter().map(|occurrence| Occurrence { component: component.to_owned(), ..occurrence }),
                );
            }
        }
        _ => {}
    }
}

/// The pointers of a payload's annotated values, as WPS's own walk finds them
/// (`wp:server/server/src/storage/references.rs:118-140`): the only places WPS indexes or respells a reference.
pub fn annotated_pointers(payload: &Value, annotations: &[RefAnnotation]) -> Vec<String> {
    annotations.iter().flat_map(|annotation| pointed(payload, &annotation.path).into_iter().map(|(at, _)| at)).collect()
}

/// Write bare every reference written `@head` at an annotated place of `components`, as WPS stores it, and say which
/// (`wp:server/server/src/storage/references.rs:268-297`); a string elsewhere stays as written, whatever it reads as.
pub fn respell_heads(
    entity: &str,
    components: &mut Map<String, Value>,
    annotations: &dyn Annotations,
) -> Vec<Respelled> {
    let mut respelled = Vec::new();
    for (component, payload) in components.iter_mut() {
        for pointer in annotated_pointers(payload, annotations.refs(component)) {
            let Some(value) = payload.pointer_mut(&pointer) else { continue };
            let Some(text) = value.as_str() else { continue };
            let Ok(reference) = text.parse::<EntityRef>() else { continue };
            if reference.pin() != Some(&RefPin::Head) {
                continue;
            }
            let stored = reference.id().to_string();
            respelled.push(Respelled {
                entity: entity.to_owned(),
                component: component.clone(),
                path: pointer,
                written: text.to_owned(),
                stored: stored.clone(),
            });
            *value = Value::String(stored);
        }
    }
    respelled
}

/// The values a pointer names: one, or each element under a trailing `/*`.
fn pointed<'a>(payload: &'a Value, pointer: &str) -> Vec<(String, &'a Value)> {
    match pointer.strip_suffix("/*") {
        None => payload.pointer(pointer).map(|value| vec![(pointer.to_owned(), value)]).unwrap_or_default(),
        Some(base) => match payload.pointer(base) {
            Some(Value::Array(items)) => {
                items.iter().enumerate().map(|(index, item)| (format!("{base}/{index}"), item)).collect()
            }
            _ => Vec::new(),
        },
    }
}

/// How the value at a reference place writes its reference, or `None` where it writes none: a null, or, inside a
/// template, a parameter its stamp replaces, `{"$param": "<name>"}` with at most a `$comment` beside it. SPEC §2 makes
/// a placeholder no reference, and WPS refuses one at the top of a component, where a reference field holds a
/// reference (`wp:server/DESIGN.md`, on placeholders), so outside a template a parameter is malformed here too.
fn form_of(value: &Value, locals: Option<&[String]>) -> Option<RefForm> {
    Some(match value {
        Value::Null => return None,
        Value::String(text) => RefForm::Text(text.clone()),
        Value::Object(fields) => {
            let text = |key: &str| fields.get(key).and_then(Value::as_str).map(str::to_owned);
            let parameter = text("$param").is_some() && fields.keys().all(|key| key == "$param" || key == "$comment");
            if let (Some(generator), Some(key)) = (text("generator"), text("key")) {
                RefForm::TwoPart { generator, key }
            } else if parameter && locals.is_some() {
                return None;
            } else if let Some(handle) = text("local_ref").or_else(|| text("local")).filter(|_| locals.is_some()) {
                let known = locals.is_some_and(|handles| handles.contains(&handle));
                RefForm::Local { handle, known }
            } else {
                RefForm::Malformed(value.clone())
            }
        }
        other => RefForm::Malformed(other.clone()),
    })
}

fn opens_with_uuid(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 36 && bytes[..8].iter().all(u8::is_ascii_hexdigit) && bytes[8] == b'-'
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{Annotations, Occurrence, RefForm, components_read, walk};
    use crate::wire::RefAnnotation;

    /// The one annotation these tests read: `storage` holds a reference at `/carrier_type`.
    struct Storage(Vec<RefAnnotation>);

    impl Annotations for Storage {
        fn refs(&self, component: &str) -> &[RefAnnotation] {
            if component == "storage" { &self.0 } else { &[] }
        }
    }

    fn storage() -> Storage {
        Storage(vec![RefAnnotation { path: "/carrier_type".to_owned(), expects: None }])
    }

    /// A templated tiling's derived map can be a placeholder a stamp fills, as Walmart DFW6's are: `$param` names no
    /// component, so a walk of the tiling reads no annotations but its own and finds no reference.
    #[test]
    fn a_placeholder_standing_for_a_derived_map_holds_no_component() {
        let tiling = json!({ "derives": { "$param": "derives" } });
        assert_eq!(components_read("tiling", &tiling), ["tiling"]);
        assert!(walk("tiling", &tiling, &storage()).is_empty());
    }

    /// A note beside a template internal's components is none of them: the components beside it are read, and their
    /// references found, as without it; and a note beside an entity's own components names nothing to read.
    #[test]
    fn a_comment_beside_components_is_none_of_them() {
        let storage_of = json!({ "holds": "inventory", "carrier_type": "0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10@1" });
        let components = json!({ "$comment": "the GO catalog's rack", "storage": storage_of });
        let template = json!({ "entities": [{ "local": "rack", "components": components }] });
        assert_eq!(components_read("template", &template), ["template", "storage"]);
        let found = walk("template", &template, &storage());
        let paths: Vec<&str> = found.iter().map(|occurrence| occurrence.path.as_str()).collect();
        assert_eq!(paths, ["/entities/0/components/storage/carrier_type"]);
        let note = Value::String("a note".to_owned());
        assert!(components_read("$comment", &note).is_empty() && walk("$comment", &note, &storage()).is_empty());
    }

    /// A parameter standing at a reference place inside a template is no reference, alone or with a note beside it,
    /// since its stamp replaces it; at the top of a component, or in any shape a stamp does not replace, it is
    /// malformed.
    #[test]
    fn a_parameter_at_a_reference_place_is_no_reference_inside_a_template_only() {
        let templated = |carrier: &Value| {
            let rack = json!({ "local": "rack", "components": { "storage": { "carrier_type": carrier } } });
            json!({ "entities": [rack] })
        };
        let malformed = |found: &[Occurrence]| matches!(found, [Occurrence { form: RefForm::Malformed(_), .. }]);
        for parameter in [json!({ "$param": "carrier" }), json!({ "$param": "carrier", "$comment": "the bay's" })] {
            assert!(walk("template", &templated(&parameter), &storage()).is_empty(), "{parameter}");
            let found = walk("storage", &json!({ "carrier_type": parameter }), &storage());
            assert!(malformed(&found), "{found:?}");
        }
        for unreplaced in
            [json!({ "$param": "carrier", "target": "0c2f6d5e-8a41-4f7e-9b13-5d2e7a9c4f10" }), json!({ "$param": 3 })]
        {
            let found = walk("template", &templated(&unreplaced), &storage());
            assert!(malformed(&found), "{found:?}");
        }
    }
}
