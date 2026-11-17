//! A template's placeholders, refused wherever one is left outside a template.
//!
//! A placeholder is what a template writes where its stamp supplies the value:
//! a parameter, `{"$param": "floor"}`; a local handle, alone or beside a key,
//! `{"local_ref": "patch", "key": "cell:1,0"}`; or an address whose key a
//! parameter supplies. SPEC §6 lets one stand inside a template's internals and
//! nowhere else, since only a stamp binds a parameter or names a handle. The
//! schema layer shares its value definitions between template internals and
//! world data, and cannot see where a value stands, so it admits a parameter
//! wherever a number, a numeric array, a flag, a rack's levels or a reference
//! may go, and a handle wherever a reference may go (`proto/schema/README.md`).
//! This module is the reader's half of that rule.
//!
//! It reads those places off the schema the reader is built with, rather than
//! off a list of fields, so a field the schema gains is covered the day it is
//! added, and a field the reader carries unread is covered like one it reads.
//! A handle is a placeholder only where a reference goes; anywhere else a value
//! of its shape is plain data, for whatever reads the field to judge. A
//! component the reader does not know is never walked: its fields mean what its
//! owner says (SPEC §9).
//!
//! [`refuse`] runs where component data enters the world, before any stage
//! reads it: on every declared entity as expansion starts, on each stamped
//! internal once its stamp has bound its parameters and named its handles, and
//! on each override as it is applied. Every other derived entity is built by a
//! generator out of values it has already read, or copies a tiling's `derives`,
//! which the walk reads as a component map on the tiling itself. So no entity
//! that survives the load holds a placeholder.

use crate::error::LoadError;
use crate::ids::{EntityId, LocalHandle};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// The schema files the reader is built with: the shared values, the register
/// of components, and the files that define them.
const SCHEMA: [&str; 7] = [
    include_str!("../../../data/seed/application/schema/values.json"),
    include_str!("../../../data/seed/application/schema/components.json"),
    include_str!("../../../data/seed/application/schema/components/spatial.json"),
    include_str!("../../../data/seed/application/schema/components/structure.json"),
    include_str!("../../../data/seed/application/schema/components/fleet.json"),
    include_str!("../../../data/seed/application/schema/components/product.json"),
    include_str!("../../../data/seed/application/schema/components/identity.json"),
];

/// The three reference types: where the schema puts one of them, outside a
/// template, any placeholder is refused, a handle as well as a parameter.
const REFERENCE_TYPES: [&str; 3] = [
    "urn:worldproto:0.7:values#/$defs/entity_ref",
    "urn:worldproto:0.7:values#/$defs/cell_address",
    "urn:worldproto:0.7:values#/$defs/derivation_key",
];

/// A parameter, which the schema lets stand for a number, a numeric array, a
/// flag or a rack's levels too: where it admits one, outside a template, a
/// parameter is refused.
const PARAMETER: &str = "urn:worldproto:0.7:values#/$defs/param_ref";

/// A component map, as an entity carries one and as a tiling's `derives` and a
/// template internal's `components` embed one.
const COMPONENT_MAP: &str = "urn:worldproto:0.7:components#/$defs/set";

/// A template's internal entity, the one place a placeholder stands.
const TEMPLATE_INTERNAL: &str = "urn:worldproto:0.7:components:product#/$defs/template_entity";

/// Refuse the first placeholder left in `components` where the schema admits
/// one, naming the entity, the component, the field and the placeholder.
pub fn refuse(entity: &EntityId, components: &BTreeMap<String, Value>) -> Result<(), LoadError> {
    let sites = sites();
    for (name, value) in components {
        sites.component(entity, name, "", name, value)?;
    }
    Ok(())
}

/// What a value is, when it is a placeholder: a parameter, a local handle
/// alone or beside a key, or an address whose key is a parameter. Anything else
/// is `None`, and is left to whatever reads the field.
pub(crate) fn describe(value: &Value) -> Option<String> {
    if let Some(parameter) = described_parameter(value) {
        return Some(parameter);
    }
    let object = value.as_object()?;
    if let Some(handle) = object.get("local_ref") {
        return Some(match object.get("key") {
            None => format!("the local handle {}", spelled(handle)),
            Some(key) => match parameter(key) {
                Some(name) => {
                    format!("the local handle {} with the parameter {} for its key", spelled(handle), spelled(name))
                }
                None => format!("the local handle {} with the key {}", spelled(handle), spelled(key)),
            },
        });
    }
    let name = object.get("key").and_then(parameter)?;
    Some(format!("an address whose key is the parameter {}", spelled(name)))
}

/// The refusal a stage that reads a typed cell address raises on a local one.
/// [`refuse`] meets every such address where it enters the world, so a stage
/// raises this only when it runs over entities that did not enter through the
/// pipeline.
pub(crate) fn unresolved_handle(entity: &EntityId, component: &str, field: String, handle: &LocalHandle) -> LoadError {
    LoadError::UnresolvedPlaceholder {
        entity: entity.clone(),
        component: component.to_string(),
        field,
        placeholder: format!("the local handle `{handle}`"),
    }
}

fn described_parameter(value: &Value) -> Option<String> {
    parameter(value).map(|name| format!("the parameter {}", spelled(name)))
}

fn parameter(value: &Value) -> Option<&Value> {
    value.as_object()?.get("$param")
}

/// A name as a refusal quotes it: a string as written, anything else as JSON.
fn spelled(value: &Value) -> String {
    match value {
        Value::String(text) => format!("`{text}`"),
        other => format!("`{other}`"),
    }
}

// ---------------------------------------------------------------------------
// Where the schema admits a placeholder
// ---------------------------------------------------------------------------

/// One place inside a component where the schema types a value we inspect.
#[derive(Debug)]
struct Site {
    /// The way down from the component's top to the value.
    steps: Vec<Step>,
    holds: Holds,
}

#[derive(Debug, Clone, PartialEq)]
enum Step {
    /// The field of this name.
    Field(String),
    /// Every element of an array.
    Each,
    /// Every field the object's schema does not name: the values of a map.
    Others(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Holds {
    /// An entity reference, a cell address or a derivation key.
    Reference,
    /// A value a parameter may stand for and a handle may not.
    Parameter,
    /// A component map, each component in it walked by its own sites.
    Components,
}

/// The sites of every component the schema's register names, by name.
struct Sites(BTreeMap<String, Vec<Site>>);

fn sites() -> &'static Sites {
    static SITES: OnceLock<Sites> = OnceLock::new();
    SITES.get_or_init(|| Schema::embedded().sites())
}

impl Sites {
    /// One component: `top` is the component the refusal names, and `field`
    /// where this one sits inside it, empty unless it is embedded.
    fn component(&self, entity: &EntityId, top: &str, field: &str, name: &str, value: &Value) -> Result<(), LoadError> {
        for site in self.0.get(name).into_iter().flatten() {
            self.follow(entity, top, field, value, &site.steps, site.holds)?;
        }
        Ok(())
    }

    fn follow(
        &self,
        entity: &EntityId,
        top: &str,
        field: &str,
        value: &Value,
        steps: &[Step],
        holds: Holds,
    ) -> Result<(), LoadError> {
        let Some((step, rest)) = steps.split_first() else {
            let placeholder = match holds {
                Holds::Reference => describe(value),
                Holds::Parameter => described_parameter(value),
                Holds::Components => {
                    for (name, inner) in value.as_object().into_iter().flatten() {
                        self.component(entity, top, &joined(field, name), name, inner)?;
                    }
                    return Ok(());
                }
            };
            return match placeholder {
                Some(placeholder) => Err(LoadError::UnresolvedPlaceholder {
                    entity: entity.clone(),
                    component: top.to_string(),
                    field: field.to_string(),
                    placeholder,
                }),
                None => Ok(()),
            };
        };
        match step {
            Step::Field(name) => match value.get(name) {
                Some(inner) => self.follow(entity, top, &joined(field, name), inner, rest, holds),
                None => Ok(()),
            },
            Step::Each => {
                for (index, inner) in value.as_array().into_iter().flatten().enumerate() {
                    self.follow(entity, top, &format!("{field}[{index}]"), inner, rest, holds)?;
                }
                Ok(())
            }
            Step::Others(named) => {
                for (name, inner) in value.as_object().into_iter().flatten() {
                    if name != "$comment" && !named.contains(name) {
                        self.follow(entity, top, &joined(field, name), inner, rest, holds)?;
                    }
                }
                Ok(())
            }
        }
    }
}

fn joined(field: &str, name: &str) -> String {
    if field.is_empty() {
        name.to_string()
    } else {
        format!("{field}.{name}")
    }
}

/// The embedded schema files, by the `$id` each registers itself under, which
/// is how they name each other.
struct Schema(BTreeMap<String, Value>);

impl Schema {
    fn embedded() -> Schema {
        let mut files = BTreeMap::new();
        for text in SCHEMA {
            let file: Value = serde_json::from_str(text).expect("every schema file the reader is built with is JSON");
            if let Some(id) = file.get("$id").and_then(Value::as_str).map(str::to_string) {
                files.insert(id, file);
            }
        }
        Schema(files)
    }

    /// A definition by its absolute reference, with the `$id` of the file it
    /// is in, against which its own references resolve.
    fn resolve(&self, reference: &str) -> Option<(&str, &Value)> {
        let (id, pointer) = reference.split_once('#')?;
        let (id, file) = self.0.get_key_value(id)?;
        Some((id, file.pointer(pointer)?))
    }

    fn sites(&self) -> Sites {
        let mut out = BTreeMap::new();
        if let Some((base, set)) = self.resolve(COMPONENT_MAP) {
            for (name, schema) in set.get("properties").and_then(Value::as_object).into_iter().flatten() {
                let mut sites = Vec::new();
                self.compile(base, schema, &mut Vec::new(), &mut Vec::new(), &mut sites);
                out.insert(name.clone(), sites);
            }
        }
        Sites(out)
    }

    /// Every site under one schema node, reached by `steps`. A reference to a
    /// definition already being followed is not followed again, so a schema
    /// that nests itself ends.
    fn compile(
        &self,
        base: &str,
        node: &Value,
        steps: &mut Vec<Step>,
        following: &mut Vec<String>,
        out: &mut Vec<Site>,
    ) {
        let Some(node) = node.as_object() else { return };
        if let Some(reference) = node.get("$ref").and_then(Value::as_str) {
            let target = if reference.starts_with('#') { format!("{base}{reference}") } else { reference.to_string() };
            if REFERENCE_TYPES.contains(&target.as_str()) {
                out.push(Site { steps: steps.clone(), holds: Holds::Reference });
            } else if target == PARAMETER {
                out.push(Site { steps: steps.clone(), holds: Holds::Parameter });
            } else if target == COMPONENT_MAP {
                out.push(Site { steps: steps.clone(), holds: Holds::Components });
            } else if target != TEMPLATE_INTERNAL && !following.contains(&target) {
                if let Some((file, definition)) = self.resolve(&target) {
                    following.push(target);
                    self.compile(file, definition, steps, following, out);
                    following.pop();
                }
            }
        }
        let properties = node.get("properties").and_then(Value::as_object);
        for (name, schema) in properties.into_iter().flatten() {
            steps.push(Step::Field(name.clone()));
            self.compile(base, schema, steps, following, out);
            steps.pop();
        }
        if let Some(items) = node.get("items") {
            steps.push(Step::Each);
            self.compile(base, items, steps, following, out);
            steps.pop();
        }
        if let Some(others) = node.get("additionalProperties") {
            steps.push(Step::Others(properties.into_iter().flatten().map(|(name, _)| name.clone()).collect()));
            self.compile(base, others, steps, following, out);
            steps.pop();
        }
        // A branch constrains the same value its node does, so its sites sit
        // where the node's do. `if` is a condition and types nothing.
        for keyword in ["anyOf", "allOf", "oneOf"] {
            for branch in node.get(keyword).and_then(Value::as_array).into_iter().flatten() {
                self.compile(base, branch, steps, following, out);
            }
        }
        for keyword in ["then", "else"] {
            if let Some(branch) = node.get(keyword) {
                self.compile(base, branch, steps, following, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The sites of one component holding a reference or a component map,
    /// spelled as paths: `.` before a field, `[]` for every element of an
    /// array, `{}` for every value of a map, and `:` after a component map.
    fn spelled_sites(component: &str) -> Vec<String> {
        paths(component, |holds| holds != Holds::Parameter)
    }

    /// The sites of one component where a parameter may stand and a handle may
    /// not, spelled the same way.
    fn parameter_sites(component: &str) -> Vec<String> {
        paths(component, |holds| holds == Holds::Parameter)
    }

    fn paths(component: &str, keep: impl Fn(Holds) -> bool) -> Vec<String> {
        sites().0[component]
            .iter()
            .filter(|site| keep(site.holds))
            .map(|site| {
                let mut path = String::new();
                for step in &site.steps {
                    match step {
                        Step::Field(name) => path.push_str(&format!(".{name}")),
                        Step::Each => path.push_str("[]"),
                        Step::Others(_) => path.push_str("{}"),
                    }
                }
                if site.holds == Holds::Components {
                    path.push(':');
                }
                path
            })
            .collect()
    }

    /// The names this walk stops at, and the map it recurses through, are
    /// definitions the schema still has: a renamed one would leave the walk
    /// matching nothing, and every reference would pass.
    #[test]
    fn the_definitions_the_walk_stops_at_are_in_the_schema() {
        let schema = Schema::embedded();
        for name in REFERENCE_TYPES.iter().chain([&PARAMETER, &COMPONENT_MAP, &TEMPLATE_INTERNAL]) {
            assert!(schema.resolve(name).is_some(), "{name} names no definition the reader is built with");
        }
    }

    /// The walk reads every schema node the component register reaches: each
    /// `$ref` resolves inside the files embedded above, so a component file the
    /// schema gains without a line there fails here; and each keyword is one the
    /// walk follows or one that types no field, so a keyword that would hide a
    /// reference beneath it, `prefixItems` or `patternProperties`, fails too.
    #[test]
    fn the_walk_reads_every_schema_node_the_register_reaches() {
        const FOLLOWED: [&str; 9] =
            ["$ref", "properties", "items", "additionalProperties", "anyOf", "allOf", "oneOf", "then", "else"];
        const TYPING_NOTHING: [&str; 17] = [
            "$comment",
            "title",
            "description",
            "type",
            "required",
            "enum",
            "const",
            "pattern",
            "format",
            "minLength",
            "minItems",
            "maxItems",
            "minimum",
            "exclusiveMinimum",
            "default",
            "if",
            "unevaluatedProperties",
        ];
        let schema = Schema::embedded();
        let mut seen = BTreeSet::new();
        let mut pending = vec![COMPONENT_MAP.to_string()];
        let mut used = BTreeSet::new();
        while let Some(target) = pending.pop() {
            if !seen.insert(target.clone()) {
                continue;
            }
            let (base, node) = schema.resolve(&target).unwrap_or_else(|| panic!("{target} resolves to nothing"));
            let mut nodes = vec![node];
            while let Some(node) = nodes.pop() {
                let Value::Object(node) = node else { continue };
                for (keyword, child) in node {
                    used.insert(keyword.as_str());
                    match keyword.as_str() {
                        "$ref" => {
                            let reference = child.as_str().expect("a $ref is a string");
                            pending.push(if reference.starts_with('#') {
                                format!("{base}{reference}")
                            } else {
                                reference.to_string()
                            });
                        }
                        "properties" => nodes.extend(child.as_object().into_iter().flatten().map(|(_, schema)| schema)),
                        "anyOf" | "allOf" | "oneOf" => nodes.extend(child.as_array().into_iter().flatten()),
                        "items" | "additionalProperties" | "then" | "else" => nodes.push(child),
                        _ => {}
                    }
                }
            }
        }
        assert!(used.contains("$ref") && used.contains("properties"), "the census read the component schemas");
        let unknown: Vec<&str> = used
            .into_iter()
            .filter(|keyword| !FOLLOWED.contains(keyword) && !TYPING_NOTHING.contains(keyword))
            .collect();
        assert!(unknown.is_empty(), "keywords the walk neither follows nor knows to type nothing: {unknown:?}");
    }

    /// The fields the review found carried unread, and fields the reader reads,
    /// each where the schema types it; a tiling's `derives` as the component
    /// map it is; and a template's internals nowhere.
    #[test]
    fn the_sites_are_where_the_schema_types_a_reference() {
        assert_eq!(spelled_sites("cell"), [".location_profile", ".zone"]);
        assert_eq!(spelled_sites("presentation_point"), [".attributes.cell", ".station"]);
        assert_eq!(spelled_sites("stitch"), [".a", ".b", ".curve.frame", ".pairs[][]"]);
        assert_eq!(spelled_sites("queue"), [".path[]"]);
        assert_eq!(spelled_sites("cohort"), [".assignment.to[]", ".robot_class"]);
        assert_eq!(spelled_sites("tiling"), [".derives:"]);
        assert_eq!(spelled_sites("template"), Vec::<String>::new());
    }

    /// A parameter may stand for a number, a numeric array or one of its
    /// elements, a flag, or a rack's levels or a number inside them, in a
    /// component the reader reads and in one it carries unread, `carrier_type`.
    /// A stamp's bindings and a template's defaults are untyped, and a
    /// template's internals are where parameters belong, so a stamp has sites
    /// only in its repeat and a template none.
    #[test]
    fn the_parameter_sites_are_wherever_the_schema_admits_one() {
        assert_eq!(parameter_sites("region"), [".origin[]", ".origin", ".size[]", ".size"]);
        assert_eq!(parameter_sites("carrier_type"), [".self_carrying", ".size[]", ".size"]);
        assert_eq!(
            parameter_sites("rack"),
            [
                ".footprint[]",
                ".footprint",
                ".levels[].height",
                ".levels[].positions[][]",
                ".levels[].positions[]",
                ".levels[].positions",
                ".levels[].reach",
                ".levels",
                ".slot_pitch[]",
                ".slot_pitch",
                ".slots_deep",
                ".slots_wide",
            ]
        );
        assert_eq!(parameter_sites("stamp"), [".repeat.count", ".repeat.step[]", ".repeat.step"]);
        assert_eq!(parameter_sites("template"), Vec::<String>::new());
    }

    #[test]
    fn each_placeholder_is_described_by_what_it_names() {
        let cases = [
            (serde_json::json!({ "$param": "floor" }), "the parameter `floor`"),
            (serde_json::json!({ "local_ref": "stand", "$comment": "c" }), "the local handle `stand`"),
            (
                serde_json::json!({ "local_ref": "patch", "key": "cell:1,0" }),
                "the local handle `patch` with the key `cell:1,0`",
            ),
            (
                serde_json::json!({ "local_ref": "patch", "key": { "$param": "queue" } }),
                "the local handle `patch` with the parameter `queue` for its key",
            ),
            (
                serde_json::json!({ "generator": "a0000000-0000-4000-8000-000000000001", "key": { "$param": "tile" } }),
                "an address whose key is the parameter `tile`",
            ),
        ];
        for (value, said) in cases {
            assert_eq!(describe(&value).as_deref(), Some(said), "{value}");
        }
        for value in [
            serde_json::json!("a0000000-0000-4000-8000-000000000001"),
            serde_json::json!({ "generator": "a0000000-0000-4000-8000-000000000001", "key": "cell:0,0" }),
            serde_json::json!([450, 900]),
        ] {
            assert_eq!(describe(&value), None, "{value}");
        }
    }
}
