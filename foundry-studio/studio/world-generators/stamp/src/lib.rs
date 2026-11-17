//! The stamp generator: a template placed somewhere becomes the instances that
//! placement stands for, and inside each one a copy of every internal the
//! template is made of.
//!
//! What it is FOR is letting a site be written as products rather than as parts.
//! An EasyLoop station is thirty entities wired to each other; a document that
//! had to write those out per station would say the same thing four hundred and
//! forty-eight times, and could get one of them wrong. So a template says it
//! once with its internals named by local handles, and a stamp says where, how
//! many, and what the parameters are worth here.
//!
//! Three rewrites turn a template internal into an entity, in this order and no
//! other: a parameter placeholder becomes the value this stamp bound to it; a
//! local handle becomes the identity that handle got in THIS instance, which is
//! what lets a template ship its own internal wiring; and the geometry — a pose,
//! a rectangle, a pitch, a cell's size — is carried out of the template's own
//! axes into the frame the stamp is posed in. Parameters go first because a
//! parameter's value may itself be a handle reference, or the key beside one,
//! and handles go before geometry because a rewritten reference is not
//! geometry.
//!
//! What it does not do is expand what it produces. An internal carrying a
//! `rack` or a `tiling` is itself a generator's input, emitted here with its
//! components and left for that generator to run over — which is the pass after
//! this one, and is why every address below an internal is another module's.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use foundry_spatial::{Degrees, Steps};
use worldgen_sdk::{
    data_keys, export_generator, marker, rewrite, root_and_key, Derived, Generator, Key, KeyError, Object,
};

/// The view this generator wants its input through: the stamp, and the template
/// it names.
///
/// The template is reached as a field rather than looked up, so the object
/// carries its whole component map and this module resolves no identity. `any`
/// rather than `template` on purpose: a stamp pointing at something that is not
/// a template must reach this generator and be refused in words, and a variable
/// that demanded the component would instead make the object quietly disappear.
/// Optional for the same reason one step further out — a stamp whose template
/// names nothing still arrives, and still gets a sentence.
const LENS: &str = r#"
// Census 46, product piece: a placed template, with the template beside it.
pattern stamped { s: stamp; t?: any; ref(s, t, "stamp.template"); }
lens stamped { anchor s: stamped; template: stamped by s -> t; }
"#;

/// One placement of a template, optionally repeated.
#[derive(Debug, Deserialize)]
struct Stamp {
    template: String,
    #[serde(default)]
    params: BTreeMap<String, Value>,
    #[serde(default)]
    repeat: Option<Repeat>,
}

/// Instance `i` sits at `pose.position + i·step`, in the frame of the stamp's
/// own pose and deliberately unrotated by its heading, so a row turned 180°
/// marches along the same world axis and spans the same extent (SPEC §6).
///
/// `u64` and not `usize`: this is compiled for `wasm32-wasip2`, where `usize`
/// is 32 bits, and the reference reader's is 64. A count above four billion
/// parses there and would be refused here while deserialising, which is a
/// document the two implementations would disagree about accepting.
#[derive(Debug, Deserialize)]
struct Repeat {
    count: u64,
    step: [Steps; 2],
}

/// A reusable sub-world: internals named by local handles instead of
/// identities, positioned in the template's own axes, parameterized where the
/// product varies.
#[derive(Debug, Deserialize)]
struct TemplateView {
    #[serde(default)]
    params: BTreeMap<String, ParamDecl>,
    #[serde(default)]
    entities: Vec<Internal>,
}

/// A parameter's declaration. One with no default must be bound by every stamp.
#[derive(Debug, Deserialize)]
struct ParamDecl {
    #[serde(default)]
    default: Option<Value>,
}

/// One entity inside a template, addressed by its handle.
#[derive(Debug, Deserialize)]
struct Internal {
    local: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    components: BTreeMap<String, Value>,
}

/// A position, and optionally a heading, in some frame (SPEC §5). Inside a
/// template the frame is absent: the pose is in the template's own axes and
/// acquires a frame when a stamp places it.
#[derive(Debug, Deserialize)]
struct Pose {
    #[serde(default)]
    frame: Option<String>,
    position: Vec<Steps>,
    #[serde(default)]
    heading_deg: Degrees,
}

impl Pose {
    /// The planar part of the position. v0 geometry is planar; a third entry is
    /// elevation and takes part in no rectangle arithmetic.
    fn xy(&self) -> Option<[Steps; 2]> {
        match self.position.len() {
            0 | 1 => None,
            _ => Some([self.position[0], self.position[1]]),
        }
    }
}

/// A rectangle anchored at its minimum corner, in the steps of whatever states
/// it (SPEC §5).
#[derive(Debug, Deserialize)]
struct Region {
    #[serde(default)]
    frame: Option<String>,
    origin: [Steps; 2],
    size: [Steps; 2],
}

/// The two fields of a tiling a quarter turn reads: the pitch it swaps and the
/// carve-outs it cannot re-corner.
#[derive(Debug, Deserialize)]
struct Tiling {
    pitch: [Steps; 2],
    #[serde(default)]
    exclusions: Vec<Value>,
}

/// Where one instance stands: everything the composition of an internal's
/// geometry needs, and nothing about the template.
struct Placement {
    frame: String,
    at: [Steps; 2],
    heading: Degrees,
}

/// The anchor this run hangs its addresses off, in the two spellings a
/// reference needs: the identity the host knows it by, and that identity parted
/// into the declared entity at its head and the chain from there down to it.
///
/// Both, because the two reference shapes root differently. A reference to an
/// internal is that internal's whole identity, which starts at the anchor. A
/// reference to something *beneath* an internal is a declared entity plus one
/// key, so the anchor's own chain moves into the key — and an anchor another
/// generator derived has one.
struct Anchored<'a> {
    identity: &'a str,
    root: &'a str,
    ancestry: Option<&'a str>,
}

struct StampGenerator;

impl Generator for StampGenerator {
    const NAME: &'static str = "stamp";
    const VERSION: &'static str = "0.1.0";
    const LENS: &'static str = LENS;
    // `pose` is what this generator states itself, on every instance. An
    // internal carries whatever its template says it carries, which is any
    // component at all, so this list is a lower bound by construction.
    const WRITES: &'static [&'static str] = &["pose"];

    fn generate(object: &Object) -> Result<Vec<Derived>, String> {
        let anchor = object.anchor();
        let stamp: Stamp = anchor.require("stamp")?;
        let pose: Pose = anchor.require("pose")?;
        let Some(frame) = pose.frame.clone() else {
            return Err(refusal(anchor.id(), "a stamp must be posed in a frame"));
        };
        let Some(at) = pose.xy() else {
            return Err(refusal(anchor.id(), "a stamp's pose needs at least a planar position"));
        };

        let Some(target) = object.field("template")?.first() else {
            return Err(format!(
                "entity {}: `stamp.template` references {}, which does not exist",
                anchor.id(),
                stamp.template
            ));
        };
        // The lens binds the template by the very reference the stamp states, so
        // a binding naming some other entity is the host disagreeing with the
        // document rather than anything the document said. Saying so stops a
        // whole template's internals being stamped under a stamp that never
        // asked for them, dressed as a fact about the site.
        // By value and not by spelling: the schema accepts a UUID in either
        // case and the reader writes identities lowercase, so the declared head
        // compares without regard to case; a derivation chain is byte-exact, as
        // the reader keys it, so that part compares as written.
        let (wanted_root, wanted_chain) = root_and_key(&stamp.template);
        let (bound_root, bound_chain) = root_and_key(target.id());
        if !wanted_root.eq_ignore_ascii_case(bound_root) || wanted_chain != bound_chain {
            let said = format!("the lens bound template {} where the stamp names {}", target.id(), stamp.template);
            return Err(refusal(anchor.id(), &said));
        }
        let template: TemplateView = target.require("template")?;
        let bindings = bind_params(&template, &stamp, anchor.id(), target.id())?;

        let (count, step) = match &stamp.repeat {
            None => (1u64, [Steps(0), Steps(0)]),
            Some(repeat) => match repeat.count {
                0 => return Err(refusal(anchor.id(), "`repeat.count` must be a positive integer")),
                count => (count, repeat.step),
            },
        };
        check_handles(&template, target.id())?;

        let (root, ancestry) = root_and_key(anchor.id());
        let anchored = Anchored { identity: anchor.id(), root, ancestry };

        let mut out = Vec::new();
        for index in 0..count {
            let placement = Placement {
                frame: frame.clone(),
                at: marched(at, step, index).ok_or_else(|| off_lattice(anchor.id()))?,
                heading: pose.heading_deg,
            };
            let instance = Key::new(format!("instance:{index}"))?;
            out.push(Derived::at(instance.clone()).with(
                "pose",
                json!({
                    "frame": placement.frame,
                    "position": placement.at,
                    "heading_deg": placement.heading,
                }),
            ));

            // Every handle resolved before any internal is built, because an
            // internal may name one that comes after it in the list. A handle
            // stands for the key the host hangs off the anchor; the identity it
            // becomes is that key under the anchor's own.
            let handles: BTreeMap<&str, Key> = template
                .entities
                .iter()
                .map(|one| Ok((one.local.as_str(), instance.child(format!("local:{}", one.local))?)))
                .collect::<Result<_, KeyError>>()?;

            for internal in &template.entities {
                let key = handles[internal.local.as_str()].clone();
                let address = format!("{}/{key}", anchored.identity);
                let mut wired = BTreeMap::new();
                for (name, stated) in &internal.components {
                    let bound = substitute(stated, &bindings, target.id())?;
                    wired.insert(name.clone(), wire(&bound, &handles, &anchored, target.id())?);
                }
                // A pose that names its own frame is already absolute, and a
                // cell's size runs along its pose's frame, so it stays as stated.
                let absolute = wired.get("pose").is_some_and(|pose| !pose["frame"].is_null());
                let mut components = BTreeMap::new();
                for (name, value) in wired {
                    let placed =
                        if name == "cell" && absolute { value } else { compose(&name, value, &placement, &address)? };
                    components.insert(name, placed);
                }
                let mut one = Derived::new(key, components);
                if let Some(name) = &internal.name {
                    one = one.named(name.clone());
                }
                out.push(one);
            }
        }
        Ok(out)
    }
}

/// Bind every parameter a template declares, and refuse a stamp that binds one
/// it does not: a mistyped parameter name that silently does nothing is exactly
/// the failure a document cannot see.
fn bind_params(
    template: &TemplateView,
    stamp: &Stamp,
    stamp_id: &str,
    template_id: &str,
) -> Result<BTreeMap<String, Value>, String> {
    let mut bindings = BTreeMap::new();
    for (name, declared) in &template.params {
        let bound = stamp.params.get(name).or(declared.default.as_ref()).ok_or_else(|| {
            format!("stamp {stamp_id} leaves the parameter `{name}` of template {template_id} unbound")
        })?;
        bindings.insert(name.clone(), bound.clone());
    }
    for name in stamp.params.keys() {
        if !template.params.contains_key(name) {
            return Err(format!("stamp {stamp_id} binds `{name}`, which template {template_id} does not declare"));
        }
    }
    Ok(bindings)
}

/// The two things a handle owes the address it is about to become: it must not
/// carry the `/` that parts one segment from the next (SPEC §3), and it must
/// name one internal, since two internals answering to one handle would be two
/// entities at one identity.
fn check_handles(template: &TemplateView, template_id: &str) -> Result<(), String> {
    for handle in template.entities.iter().map(|one| &one.local) {
        if handle.contains('/') {
            return Err(format!(
                "template {template_id}: the local handle `{handle}` contains `/`, which SPEC §3 \
                 reserves"
            ));
        }
        if template.entities.iter().filter(|one| &one.local == handle).count() > 1 {
            return Err(format!("template {template_id} defines the local handle `{handle}` twice"));
        }
    }
    Ok(())
}

/// Where instance `index` of a repeated stamp stands.
fn marched(at: [Steps; 2], step: [Steps; 2], index: u64) -> Option<[Steps; 2]> {
    let index = i64::try_from(index).ok()?;
    Some([at[0].checked_add(step[0].checked_mul(index)?)?, at[1].checked_add(step[1].checked_mul(index)?)?])
}

/// Replace every `{"$param": "name"}` with what this stamp bound to that name.
///
/// Nothing looks inside strings: partial interpolation does not exist in this
/// format, so a parameter stands as a whole value or not at all.
fn substitute(value: &Value, bindings: &BTreeMap<String, Value>, template_id: &str) -> Result<Value, String> {
    rewrite(value, &mut |map| {
        let name = marker(map, "$param")?;
        Some(bindings.get(name).cloned().ok_or_else(|| {
            format!(
                "template {template_id} references the parameter `{name}`, which it does not \
                 declare"
            )
        }))
    })
}

/// Turn every local handle into the identity it got in this instance.
///
/// Two shapes, because a template wires two kinds of thing: `{"local_ref": h}`
/// names the internal itself, and `{"local_ref": h, "key": k}` names something
/// that internal will itself derive — a cell of its tiling, a slot of its rack.
///
/// The second is written as a declared entity plus one key, which is how a
/// reader reads a derived address: everything from the declared entity at the
/// head of the anchor's own chain down to the key the template asked for goes
/// into the key, and the head goes into `generator`. For a stamp a document
/// declares, that head is the stamp and the chain is what this instance adds.
/// For a stamp some other generator derived, the head is further out and the
/// anchor's own chain rides in the key with the rest — naming the anchor itself
/// would name an entity no document declares.
fn wire(value: &Value, handles: &BTreeMap<&str, Key>, anchored: &Anchored, template_id: &str) -> Result<Value, String> {
    rewrite(value, &mut |map| {
        if let Some(handle) = marker(map, "local_ref") {
            return Some(
                handles
                    .get(handle)
                    .map(|key| Value::String(format!("{}/{key}", anchored.identity)))
                    .ok_or_else(|| unknown_handle(template_id, handle)),
            );
        }
        if let Some((handle, key)) = local_address(map) {
            let Some(under) = handles.get(handle) else {
                return Some(Err(unknown_handle(template_id, handle)));
            };
            let beneath = match Key::parse(key) {
                Ok(parsed) => parsed,
                Err(reason) => return Some(Err(reason.to_string())),
            };
            let chain = match anchored.ancestry {
                Some(above) => format!("{above}/{under}/{beneath}"),
                None => format!("{under}/{beneath}"),
            };
            return Some(Ok(json!({ "generator": anchored.root, "key": chain })));
        }
        // A `local_ref` in any other company is a shape nothing will rewrite,
        // and a handle that survives expansion stands for nothing. Refusing
        // names the template; letting it through would strand it.
        map.contains_key("local_ref").then(|| {
            Err(format!(
                "template {template_id}: a `local_ref` stands alone or carries exactly a `key` \
                 beside it, each holding a string; this one sits among {}",
                map.keys().map(String::as_str).collect::<Vec<_>>().join(", ")
            ))
        })
    })
}

/// The address form `{"local_ref": handle, "key": "cell:0,0"}`: the handle, and
/// the key it addresses beneath it.
fn local_address(map: &Map<String, Value>) -> Option<(&str, &str)> {
    if data_keys(map) != ["key", "local_ref"] {
        return None;
    }
    Some((map.get("local_ref")?.as_str()?, map.get("key")?.as_str()?))
}

/// Carry a template internal's local geometry into the world its instance sits
/// in. Only the components that *are* geometry move: a pose rotates and
/// translates, a rectangle is re-cornered, a pitch and a cell's size swap axes
/// under a quarter turn. Every other component, known or not, passes through
/// untouched.
fn compose(component: &str, value: Value, placement: &Placement, entity: &str) -> Result<Value, String> {
    match component {
        "pose" => compose_pose(value, placement, entity),
        "region" => compose_region(value, placement, entity),
        "tiling" => compose_tiling(value, placement, entity),
        "cell" => compose_cell(value, placement, entity),
        _ => Ok(value),
    }
}

fn compose_pose(mut value: Value, placement: &Placement, entity: &str) -> Result<Value, String> {
    let local: Pose = shaped(&value, entity, "pose")?;
    // An internal that names its own frame is already absolute; only a frameless
    // pose is local to the template.
    if local.frame.is_some() {
        return Ok(value);
    }
    let Some(xy) = local.xy() else {
        return Err(refusal(entity, "a template internal's pose needs at least a planar position"));
    };
    let position = placement.heading.place(xy, placement.at).ok_or_else(|| off_lattice(entity))?;
    let Some(object) = value.as_object_mut() else {
        return Err(refusal(entity, "`pose` must be an object"));
    };
    object.insert("frame".into(), json!(placement.frame));
    object.insert("position".into(), json!(position));
    object.insert("heading_deg".into(), json!(placement.heading.compose(local.heading_deg)));
    Ok(value)
}

fn compose_region(mut value: Value, placement: &Placement, entity: &str) -> Result<Value, String> {
    let local: Region = shaped(&value, entity, "region")?;
    if local.frame.is_some() {
        return Ok(value);
    }
    // A region is an axis-aligned rectangle, so it survives a quarter turn and
    // nothing else. Refusing the general case is deliberate: rotating a
    // rectangle by 30° makes a shape v0 has no way to write down, and keeping
    // the unrotated extents would put cells where no floor is.
    if placement.heading.quarter_turns().is_none() {
        return Err(refusal(
            entity,
            &format!(
                "a template region cannot be stamped at {}: v0 regions are axis-aligned \
                 rectangles, so only quarter-turn headings are defined",
                placement.heading
            ),
        ));
    }
    let far = [
        local.origin[0].checked_add(local.size[0]).ok_or_else(|| off_lattice(entity))?,
        local.origin[1].checked_add(local.size[1]).ok_or_else(|| off_lattice(entity))?,
    ];
    let corners = [
        placement.heading.place(local.origin, placement.at).ok_or_else(|| off_lattice(entity))?,
        placement.heading.place(far, placement.at).ok_or_else(|| off_lattice(entity))?,
    ];
    let mut origin = [Steps(0); 2];
    let mut size = [Steps(0); 2];
    for axis in 0..2 {
        origin[axis] = corners[0][axis].min(corners[1][axis]);
        let span = corners[1][axis].checked_sub(corners[0][axis]).ok_or_else(|| off_lattice(entity))?;
        size[axis] = Steps(span.0.checked_abs().ok_or_else(|| off_lattice(entity))?);
    }
    let Some(object) = value.as_object_mut() else {
        return Err(refusal(entity, "`region` must be an object"));
    };
    object.insert("frame".into(), json!(placement.frame));
    object.insert("origin".into(), json!(origin));
    object.insert("size".into(), json!(size));
    Ok(value)
}

fn compose_tiling(mut value: Value, placement: &Placement, entity: &str) -> Result<Value, String> {
    let Some(quarter) = placement.heading.quarter_turns() else {
        return Err(refusal(
            entity,
            &format!(
                "a template tiling cannot be stamped at {}: only quarter-turn headings are \
                 defined for axis-aligned pitches",
                placement.heading
            ),
        ));
    };
    // At heading 0 the region only translates, and a carve-out stated relative
    // to that region's origin translates with it: nothing to do here at all.
    if quarter == 0 {
        return Ok(value);
    }
    let local: Tiling = shaped(&value, entity, "tiling")?;
    // Every other quarter turn — 180° included, which flips an origin-relative
    // rectangle onto the far corner — would need the region's extent to say
    // where the carve-out lands, and composing one component at a time cannot
    // see it. Refusing beats stamping a hole in the wrong place.
    if !local.exclusions.is_empty() {
        return Err(refusal(
            entity,
            &format!(
                "a template tiling carrying `exclusions` cannot be stamped at {}: its carve-outs \
                 are stated from the region's origin and re-cornering them needs the region's \
                 extent, so v0 stamps such a tiling at 0° only",
                placement.heading
            ),
        ));
    }
    if quarter % 2 == 0 {
        return Ok(value);
    }
    let Some(object) = value.as_object_mut() else {
        return Err(refusal(entity, "`tiling` must be an object"));
    };
    object.insert("pitch".into(), json!([local.pitch[1], local.pitch[0]]));
    // The size `derives` gives this tiling's cells is an extent beside the
    // pitch, and turns with it.
    if let Some(cell) = object.get_mut("derives").and_then(|derives| derives.get_mut("cell")) {
        *cell = compose_cell(cell.take(), placement, entity)?;
    }
    Ok(value)
}

/// A cell's `size` is its extent along the axes it is stated in, as a pitch is
/// (SPEC §5), so a stamp turns a template's cell size the way it turns a pitch:
/// the two entries swap under an odd quarter turn (§6). Any other heading is
/// refused while a size is stated, for the reason a region's is: v0 writes an
/// extent along two axes and has no way to write one turned between them. A
/// cell stating no size has nothing to turn.
fn compose_cell(mut value: Value, placement: &Placement, entity: &str) -> Result<Value, String> {
    let Some(stated) = value.get("size").filter(|size| !size.is_null()) else {
        return Ok(value);
    };
    let Some(quarter) = placement.heading.quarter_turns() else {
        return Err(refusal(
            entity,
            &format!(
                "a template cell stating a `size` cannot be stamped at {}: v0 cell extents are \
                 axis-aligned, so only quarter-turn headings are defined",
                placement.heading
            ),
        ));
    };
    if quarter % 2 == 0 {
        return Ok(value);
    }
    let size: [Steps; 2] = shaped(stated, entity, "cell")?;
    value["size"] = json!([size[1], size[0]]);
    Ok(value)
}

fn shaped<T: DeserializeOwned>(value: &Value, entity: &str, component: &str) -> Result<T, String> {
    serde_json::from_value(value.clone())
        .map_err(|source| format!("entity {entity}: component `{component}` does not parse: {source}"))
}

fn refusal(entity: &str, message: &str) -> String {
    format!("entity {entity}: {message}")
}

fn unknown_handle(template_id: &str, handle: &str) -> String {
    format!("template {template_id} references the local handle `{handle}`, which it does not define")
}

fn off_lattice(entity: &str) -> String {
    refusal(entity, "a stamped position runs off the step lattice — the placement does not fit in a step count")
}

export_generator!(StampGenerator);
