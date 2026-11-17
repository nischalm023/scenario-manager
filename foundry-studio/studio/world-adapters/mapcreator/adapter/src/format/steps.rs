//! Within `format`, `steps` is the **last pass over the finished
//! entities**: it decides what one step of the site frame is worth and rewrites
//! every length the import wrote as a whole number of them.
//!
//! It exists because a Map Creator export measures in millimetres and the
//! protocol counts steps (SPEC §5). Doing the conversion once at the end means
//! no emission site has to know the unit, and the unit can be chosen from the
//! values a particular site actually turned up rather than declared in advance.
//!
//! Two rules, and the second is the one worth reading. The unit is the coarsest
//! that states every length exactly — **no finer than a tenth of a
//! millimetre**, because that is where a drawing stops and a finer step would
//! be pretending to a precision the survey does not have. And **exactly means
//! exactly**: a step states a length when counting the length in steps and
//! carrying the count back gives the very number the export wrote, with no
//! tolerance. A length no step states, or two that no one step states together,
//! is refused by name rather than rounded onto a step, because a survey moved
//! onto a grid is a survey changed with nobody told.
//!
//! A frame's `transform_to_parent` is converted and never rounded onto the step.
//! It is a real-valued affine map — the import recovers it by regression over the
//! site's own cells, because the export states a projection where it should state
//! a transform — and putting its intercept on the step would move every
//! coordinate projected through it. But a reader takes a translation in the
//! parent's steps, and an `index` frame's scale too: SPEC §4's lattice frame
//! "steps 750 mm per index, written as 7 500 steps of a tenth of a millimeter".
//! So both are carried into steps by the same factor as every length, and kept
//! to a thousandth of a step, which is finer than the thousandth of a millimetre
//! the fit is taken to. A scale between two length frames is a ratio, the same
//! in any unit, and stays as it is.
//!
//! What is a length is said by component and field, never by a key's name: a
//! component of the vocabulary carries what the export states, in the export's own
//! units, and a key called `position` or `size` inside one comes back as it
//! went in. A cell's footprint is the one length the export does not state
//! itself: it is the sum of two half-extents on each axis, so the import hands
//! each axis over as those two parts, each is held to the rule as the number the
//! export wrote, and the footprint is counted as the sum of their counts. Their
//! sum in binary floating point need not be the decimal sum, so holding the sum
//! to the rule would refuse two tenths that add up to a tenth.

use serde_json::Value;

/// The units a site's document may declare, coarsest first. The list stops at a
/// tenth of a millimetre on purpose: see the module doc.
const CANDIDATES: [f64; 4] = [1.0, 0.5, 0.25, 0.1];

/// Whether a site frame's `unit_mm` is one of the steps a length is counted in
/// and carried back from exactly, which both directions hold a site to.
pub fn is_step(unit_mm: f64) -> bool {
    CANDIDATES.contains(&unit_mm)
}

/// The lengths the import writes, by component and field: a pose's position and
/// a cell's footprint, which the import hands over as the half-extents each
/// axis sums (see the module doc). A frame's transform is the one other
/// geometry the import writes, and it is converted rather than counted.
const LENGTHS: [(&str, &str); 2] = [("pose", "position"), ("cell", "size")];

/// One length the export states, in millimetres, with the place among the
/// entities of the entity the import wrote it on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    pub mm: f64,
    pub entity: usize,
}

/// Why an export's lengths cannot be counted in one step of the site.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unkept {
    /// Lengths no step this pass counts in states exactly, so the site could
    /// state none of them without rounding it: how many there are, the one a
    /// tenth of a millimetre would move furthest, and what it would be counted
    /// as there.
    Unstated { count: usize, worst: Length, counted_mm: f64 },
    /// Two lengths no one step states together: the first a tenth of a
    /// millimetre cannot state, the coarsest step that states it, and the
    /// first length that step cannot state.
    NoOneStep { one: Length, step_mm: f64, other: Length },
    /// The export states a length the site's step cannot state exactly, though
    /// another step would.
    Finer { length: Length, held_mm: f64 },
    /// The site's step is not one of the units this pass counts in, so no
    /// length could be carried back into millimetres exactly.
    Unknown { held_mm: f64 },
}

/// Choose the unit, rewrite the entities in it, and answer it.
///
/// A site that already states a unit keeps it whenever it states every length
/// of this export exactly, finer than it needs or not: other systems' lengths
/// in the site are counted in it too, and a new unit would rescale them with
/// nobody told. An export that needs a finer one is refused instead. A site
/// that states none takes the coarsest unit that states every length. A length
/// no unit states, and two lengths no one unit states together, are refused
/// whatever the site holds.
pub fn to_steps(entities: &mut Value, held: Option<f64>) -> Result<f64, Unkept> {
    if let Some(held_mm) = held.filter(|held_mm| !is_step(*held_mm)) {
        return Err(Unkept::Unknown { held_mm });
    }
    let mut lengths = Vec::new();
    collect(entities, &mut lengths);
    let finest = CANDIDATES[CANDIDATES.len() - 1];
    let unstated: Vec<Length> = lengths.iter().copied().filter(|length| coarsest(length.mm).is_none()).collect();
    let moved = |length: &Length| (millimetres(count(length.mm, finest), finest) - length.mm).abs();
    let worst =
        unstated.iter().copied().reduce(|worst, length| if moved(&length) > moved(&worst) { length } else { worst });
    if let Some(worst) = worst {
        let counted_mm = millimetres(count(worst.mm, finest), finest);
        return Err(Unkept::Unstated { count: unstated.len(), worst, counted_mm });
    }
    let common = CANDIDATES.iter().copied().find(|unit| lengths.iter().all(|length| exact(length.mm, *unit)));
    let Some(common) = common else { return Err(no_one_step(&lengths, finest)) };
    let unit_mm = match held {
        Some(held_mm) => {
            if let Some(&length) = lengths.iter().find(|length| !exact(length.mm, held_mm)) {
                return Err(Unkept::Finer { length, held_mm });
            }
            held_mm
        }
        None => common,
    };
    rewrite(entities, unit_mm);
    Ok(unit_mm)
}

/// Two lengths no one step states together, when every length lands on some
/// step and none lands on all of them: a quarter of a millimetre beside a
/// tenth, since a tenth states everything a coarser step does but a quarter.
fn no_one_step(lengths: &[Length], finest: f64) -> Unkept {
    let named = || {
        let one = lengths.iter().copied().find(|length| !exact(length.mm, finest))?;
        let step_mm = coarsest(one.mm)?;
        let other = lengths.iter().copied().find(|length| !exact(length.mm, step_mm))?;
        Some(Unkept::NoOneStep { one, step_mm, other })
    };
    named().expect("with no step in common, a tenth misses one length, and the step that length needs misses another")
}

/// The coarsest step this pass counts in that states `mm` exactly.
fn coarsest(mm: f64) -> Option<f64> {
    CANDIDATES.iter().copied().find(|unit| exact(mm, *unit))
}

/// How many steps of `unit_mm` one millimetre is: 1, 2, 4 or 10 for the units a
/// site may take, each a whole number, so that a length carried into steps and
/// back is multiplied and divided by an exact factor rather than by a binary
/// approximation of a tenth.
pub fn steps_per_mm(unit_mm: f64) -> f64 {
    (1.0 / unit_mm).round()
}

/// A length in millimetres as the whole number of steps of `unit_mm` nearest
/// it, a tie going up: SPEC §5's one rounding rule, `floor(v + 0.5)`.
pub fn count(mm: f64, unit_mm: f64) -> f64 {
    (mm * steps_per_mm(unit_mm) + 0.5).floor()
}

/// A number of steps of `unit_mm` carried back into millimetres, divided by
/// the whole number of them a millimetre holds, so a count of tenths comes
/// back as the decimal it was written as.
pub fn millimetres(steps: f64, unit_mm: f64) -> f64 {
    steps / steps_per_mm(unit_mm)
}

/// Whether a step of `unit` states `mm` exactly: counted in steps and carried
/// back, it is the number the export wrote. No tolerance. Two doubles are equal
/// exactly when their canonical spellings are, each being the shortest spelling
/// that reads back as that double, so this asks of the numbers what comparing
/// the spellings would.
fn exact(mm: f64, unit: f64) -> bool {
    millimetres(count(mm, unit), unit) == mm
}

/// Every length the export states that the import wrote, found by component
/// and field, with the place of the entity it is on: every number of the
/// field, a footprint's parts included. `node` is a list of entity records,
/// each `{id, name, components}`, or a document holding one.
fn collect(node: &Value, out: &mut Vec<Length>) {
    let entities = node.as_array().or_else(|| node.get("entities").and_then(Value::as_array));
    for (entity, record) in entities.into_iter().flatten().enumerate() {
        let Some(components) = record.get("components").and_then(Value::as_object) else { continue };
        for (component, field) in LENGTHS {
            if let Some(value) = components.get(component).and_then(|c| c.get(field)) {
                numbers(value, entity, out);
            }
        }
    }
}

fn numbers(node: &Value, entity: usize, out: &mut Vec<Length>) {
    match node {
        Value::Number(number) => {
            if let Some(mm) = number.as_f64() {
                out.push(Length { mm, entity });
            }
        }
        Value::Array(items) => items.iter().for_each(|item| numbers(item, entity, out)),
        _ => {}
    }
}

/// Every length [`collect`] finds rewritten in steps of `unit`, and every
/// frame's transform carried into them.
fn rewrite(node: &mut Value, unit: f64) {
    let entities = match node {
        Value::Array(items) => Some(items),
        Value::Object(document) => document.get_mut("entities").and_then(Value::as_array_mut),
        _ => None,
    };
    for record in entities.into_iter().flatten() {
        let Some(components) = record.get_mut("components").and_then(Value::as_object_mut) else { continue };
        for (component, field) in LENGTHS {
            if let Some(value) = components.get_mut(component).and_then(|c| c.get_mut(field)) {
                scale(value, unit);
            }
        }
        if let Some(frame) = components.get_mut("frame").and_then(Value::as_object_mut) {
            let lattice = frame.get("units").and_then(Value::as_str) == Some("index");
            if let Some(transform) = frame.get_mut("transform_to_parent") {
                convert_transform(transform, unit, lattice);
            }
        }
    }
}

/// One frame's transform carried into steps: its translation always, its scale
/// when the frame's own step is a lattice index rather than a length.
fn convert_transform(transform: &mut Value, unit: f64, lattice: bool) {
    let factor = steps_per_mm(unit);
    let Value::Object(transform) = transform else { return };
    for (key, value) in transform.iter_mut() {
        if key == "translation" || (key == "scale" && lattice) {
            multiply(value, factor);
        }
    }
}

/// Every number in `node` multiplied by `factor`, to a thousandth. The fit is
/// taken to a thousandth of a millimetre, so on a millimetre site this changes
/// no number, and a transform read back off a site and written again is the
/// same number however many times it goes round.
fn multiply(node: &mut Value, factor: f64) {
    match node {
        Value::Number(number) => {
            if let Some(value) = number.as_f64() {
                let steps = (value * factor * 1000.0).round() / 1000.0;
                *node = if steps.fract() == 0.0 && steps.abs() < 9.0e15 {
                    Value::from(steps as i64)
                } else {
                    Value::from(steps)
                };
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| multiply(item, factor)),
        _ => {}
    }
}

/// Every number of one length counted in steps of `unit`, and each list of
/// parts inside it, a footprint's two half-extents on one axis, counted as the
/// sum of its parts' counts.
///
/// Every number that reaches here is a whole number of steps already, by the
/// checks in [`to_steps`], so [`count`]'s rounding decides nothing here; it is
/// SPEC §5's rule all the same, so that the count is the one the protocol's own
/// rounding gives.
fn scale(node: &mut Value, unit: f64) {
    match node {
        Value::Number(number) => {
            if let Some(mm) = number.as_f64() {
                *node = Value::from(count(mm, unit) as i64);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                match item {
                    Value::Array(parts) => {
                        let steps: f64 = parts.iter().filter_map(Value::as_f64).map(|part| count(part, unit)).sum();
                        *item = Value::from(steps as i64);
                    }
                    other => scale(other, unit),
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A key named like a length inside a component the export states whole is
    /// not a length the import wrote: it neither chooses the unit nor moves.
    #[test]
    fn only_the_lengths_the_import_writes_are_converted() {
        let mut document = json!({ "entities": [
            { "components": { "pose": { "position": [100.0, 200.0] },
                              "cell": { "barcode_properties": { "position": [1.25, 2] },
                                        "size_info": [625.05, 625, 625, 625] } } },
        ] });
        let unit = to_steps(&mut document, None).expect("a fresh site chooses");
        assert_eq!(unit, 1.0);
        let cell = &document["entities"][0]["components"]["cell"];
        assert_eq!(cell["barcode_properties"]["position"], json!([1.25, 2]));
        assert_eq!(cell["size_info"], json!([625.05, 625, 625, 625]));
    }

    /// A site that states a unit keeps it while it states every length, even
    /// where a coarser one would do; an export needing a finer one, and a unit
    /// this pass cannot count in, are refused rather than rescaling the site.
    #[test]
    fn a_held_unit_is_kept_or_the_export_refused() {
        let positions = |position: Value| json!([{ "components": { "pose": { "position": position } } }]);
        let mut whole = positions(json!([11475.0, -3825.0]));
        let unit = to_steps(&mut whole, Some(0.1)).expect("a tenth states a whole millimetre");
        assert_eq!(unit, 0.1);
        assert_eq!(whole[0]["components"]["pose"]["position"], json!([114750, -38250]));
        let mut finer = positions(json!([11475.5, -3825.0]));
        assert_eq!(
            to_steps(&mut finer, Some(1.0)).err(),
            Some(Unkept::Finer { length: Length { mm: 11475.5, entity: 0 }, held_mm: 1.0 })
        );
        assert_eq!(to_steps(&mut finer, Some(0.3)).err(), Some(Unkept::Unknown { held_mm: 0.3 }));
    }

    /// Every site seen so far measures in whole millimetres, and this is what
    /// that looks like: the coarsest unit, nothing rounded, numbers unchanged.
    #[test]
    fn a_survey_in_whole_millimetres_keeps_its_numbers() {
        let mut document = json!({ "entities": [
            { "components": { "pose": { "position": [11475.0, -3825.0] },
                              "cell": { "size": [900.0, 900.0] } } },
        ] });
        let unit = to_steps(&mut document, None).expect("a fresh site chooses");
        assert_eq!(unit, 1.0);
        let cell = &document["entities"][0]["components"];
        assert_eq!(cell["pose"]["position"], json!([11475, -3825]));
        assert_eq!(cell["cell"]["size"], json!([900, 900]));
    }

    /// A tenth of a millimetre is a step, so a survey that carries tenths keeps
    /// every one of them.
    #[test]
    fn tenths_take_the_document_to_a_tenth_step() {
        let mut document = json!({ "entities": [
            { "components": { "pose": { "position": [11475.3, -3825.0] } } },
        ] });
        let unit = to_steps(&mut document, None).expect("a fresh site chooses");
        assert_eq!(unit, 0.1);
        assert_eq!(document["entities"][0]["components"]["pose"]["position"], json!([114753, -38250]));
    }

    /// A length no step states is refused, never rounded: a hundredth, and a
    /// whole millimetre's or a tenth's binary neighbour, which any tolerance
    /// would let through. The refusal counts them and names the one a tenth
    /// would move furthest, with what it would be counted as.
    #[test]
    fn a_length_no_step_states_is_refused_naming_the_worst() {
        let positions = |position: Value| json!([{ "components": { "pose": { "position": position } } }]);
        let unstated = |position: Value, held: Option<f64>| to_steps(&mut positions(position), held).err();
        assert_eq!(
            unstated(json!([100.04, 100.07]), None),
            Some(Unkept::Unstated { count: 2, worst: Length { mm: 100.04, entity: 0 }, counted_mm: 100.0 })
        );
        for (near, step) in [(28750.0000001, 28750.0), (28750.000000000004, 28750.0), (28750.300000000003, 28750.3)] {
            for held in [None, Some(1.0), Some(0.1)] {
                assert_eq!(
                    unstated(json!([near, 0]), held),
                    Some(Unkept::Unstated { count: 1, worst: Length { mm: near, entity: 0 }, counted_mm: step }),
                    "{near} into a site holding {held:?}"
                );
            }
        }
    }

    /// Two lengths no one step states together are refused naming both: a
    /// quarter of a millimetre, which only a step of 0.25 mm states, beside a
    /// tenth, which that step does not.
    #[test]
    fn a_quarter_beside_a_tenth_is_refused_naming_both() {
        let mut document = json!([
            { "components": { "pose": { "position": [2250.25, 0] } } },
            { "components": { "pose": { "position": [3500.8, 0] } } },
        ]);
        assert_eq!(
            to_steps(&mut document, None).err(),
            Some(Unkept::NoOneStep {
                one: Length { mm: 2250.25, entity: 0 },
                step_mm: 0.25,
                other: Length { mm: 3500.8, entity: 1 }
            })
        );
    }

    /// A footprint handed over as the two half-extents each axis sums is
    /// counted as the sum of their counts. Two tenths whose sum in binary is no
    /// tenth (`0.1 + 0.2` is `0.30000000000000004`) are each a tenth, and the
    /// footprint is three of them.
    #[test]
    fn a_footprint_is_counted_as_the_sum_of_its_half_extents() {
        let mut document = json!([
            { "components": { "pose": { "position": [11475.3, 0] }, "cell": { "size": [[0.1, 0.2], [625.5, 625.5]] } } },
        ]);
        assert_eq!(to_steps(&mut document, None), Ok(0.1));
        assert_eq!(document[0]["components"]["cell"]["size"], json!([3, 12510]));
    }

    /// The half-up rule where a tie is actually representable: a whole-number
    /// step and a value sitting exactly between two of them goes up, not down,
    /// and not to the even neighbour.
    #[test]
    fn an_exact_tie_goes_up() {
        assert_eq!(rounded(2.5, 1.0), 3);
        assert_eq!(rounded(3.5, 1.0), 4);
        assert_eq!(rounded(-2.5, 1.0), -2);
    }

    fn rounded(value: f64, unit: f64) -> i64 {
        let mut node = json!(value);
        scale(&mut node, unit);
        node.as_i64().expect("an integer count of steps")
    }

    /// A frame's recovered transform is a real-valued affine map and stays one:
    /// on a millimetre site nothing about it moves.
    #[test]
    fn a_frames_transform_is_unchanged_on_a_millimetre_site() {
        let mut document = json!({ "entities": [
            { "components": { "frame": { "units": "index", "transform_to_parent": {
                "translation": [-279.745, 3575.244], "scale": [772.61, -383.691] } } } },
            { "components": { "pose": { "position": [100.0, 200.0] } } },
        ] });
        to_steps(&mut document, None).expect("a fresh site chooses");
        let transform = &document["entities"][0]["components"]["frame"]["transform_to_parent"];
        assert_eq!(transform["translation"], json!([-279.745, 3575.244]));
        assert_eq!(transform["scale"], json!([772.61, -383.691]));
    }

    /// On a site that counts tenths of a millimetre, a translation is read in
    /// the parent's steps, so it is carried into tenths; a lattice frame's scale
    /// is a length per index and is carried too; a scale between two length
    /// frames is a ratio and is not.
    #[test]
    fn a_tenths_site_carries_translations_and_a_lattice_scale_into_steps() {
        let mut document = json!({ "entities": [
            { "components": { "frame": { "units": "index", "transform_to_parent": {
                "translation": [-279.745, 3575.244], "scale": [772.61, -383.691] } } } },
            { "components": { "frame": { "units": "mm", "transform_to_parent": {
                "translation": [-1000, 2000], "scale": [1, -1] } } } },
            { "components": { "pose": { "position": [100.3, 200.0] } } },
        ] });
        let unit = to_steps(&mut document, None).expect("a fresh site chooses");
        assert_eq!(unit, 0.1);
        let lattice = &document["entities"][0]["components"]["frame"]["transform_to_parent"];
        assert_eq!(lattice["translation"], json!([-2797.45, 35752.44]));
        assert_eq!(lattice["scale"], json!([7726.1, -3836.91]));
        let vendor = &document["entities"][1]["components"]["frame"]["transform_to_parent"];
        assert_eq!(vendor["translation"], json!([-10000, 20000]));
        assert_eq!(vendor["scale"], json!([1, -1]));
        assert_eq!(document["entities"][2]["components"]["pose"]["position"], json!([1003, 2000]));
    }
}
