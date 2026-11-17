//! Within `templates`, `declarations` are what a site template declares for people (spec 05 sections 3.2 and 6): its
//! card (`template.description`), its knobs in the order the form shows them, the names of its flow's material
//! edges, and whether a value is one a knob admits. The engine holds the knobs' checks and expressions, a default
//! expression's among them (D120); these are the panels' and the PATCH route's.

use rustc_hash::FxHashMap;
use serde::Deserialize;
use studio_protocol::{KnobDeclaration, KnobDefault, KnobOption, KnobUnit, KnobValue, ScopePath, TemplateDescription};
use wps_client::ScopeSnapshot as ClientSnapshot;

use super::read::ReadFault;

fn root<'a>(template: &ScopePath, own: &'a [ClientSnapshot]) -> Result<&'a ClientSnapshot, ReadFault> {
    own.iter()
        .find(|scope| &scope.scope == template)
        .ok_or_else(|| ReadFault::Template(format!("{template} was not read with its scopes")))
}

pub fn description_of(template: &ScopePath, own: &[ClientSnapshot]) -> Result<TemplateDescription, ReadFault> {
    root(template, own)?
        .entities
        .iter()
        .find_map(|entity| entity.components.get("template.description"))
        .map(|card| serde_json::from_value(card.clone()))
        .transpose()
        .map_err(|fault| ReadFault::Template(format!("{template}'s template.description: {fault}")))?
        .ok_or_else(|| ReadFault::Template(format!("{template} holds no template.description")))
}

/// `template.knob` as a declaration the panels read; its checks are the engine's.
#[derive(Deserialize)]
struct RawKnob {
    key: String,
    label: String,
    unit: KnobUnit,
    min: Option<KnobValue>,
    max: Option<KnobValue>,
    step: Option<KnobValue>,
    #[serde(default)]
    options: Vec<KnobOption>,
    default: KnobDefault,
    description: Option<String>,
    group: Option<String>,
    order: Option<i64>,
}

pub fn knobs_of(template: &ScopePath, own: &[ClientSnapshot]) -> Result<Vec<KnobDeclaration>, ReadFault> {
    let mut knobs = Vec::new();
    for entity in &root(template, own)?.entities {
        let Some(knob) = entity.components.get("template.knob") else { continue };
        let raw: RawKnob = serde_json::from_value(knob.clone())
            .map_err(|fault| ReadFault::Template(format!("{template}'s knob {}: {fault}", entity.id)))?;
        knobs.push(KnobDeclaration {
            key: raw.key,
            label: raw.label,
            unit: raw.unit,
            min: raw.min,
            max: raw.max,
            step: raw.step,
            options: raw.options,
            default: raw.default,
            description: raw.description,
            group: raw.group,
            order: raw.order,
        });
    }
    // A group stands where its lowest `order` puts it, two groups at one order by name; a knob by its `order`, then
    // its key, within its group, so the form does not follow the order a scope's export lists entities in.
    let mut lowest: FxHashMap<Option<String>, i64> = FxHashMap::default();
    for knob in &knobs {
        let at = lowest.entry(knob.group.clone()).or_insert(i64::MAX);
        *at = (*at).min(knob.order.unwrap_or(i64::MAX));
    }
    knobs.sort_by_cached_key(|knob| {
        (lowest[&knob.group], knob.group.clone(), knob.order.unwrap_or(i64::MAX), knob.key.clone())
    });
    Ok(knobs)
}

pub fn flows_of(own: &[ClientSnapshot]) -> Vec<String> {
    let mut flows: Vec<String> = own
        .iter()
        .filter(|scope| scope.scope.slugs().last() == Some("flow"))
        .flat_map(|scope| scope.entities.iter())
        .filter(|entity| entity.components.contains_key("flow.material_edge"))
        .filter_map(|entity| entity.name.clone())
        .collect();
    flows.sort();
    flows
}

/// Whether `value` is one of `knob`'s valid values, and if not, why in words the knob's field shows; the step is
/// tested by remainders, as the engine tests it, so no difference overflows.
pub fn admits(knob: &KnobDeclaration, value: KnobValue) -> Result<(), String> {
    if !knob.options.is_empty() {
        if knob.options.iter().any(|option| option.value == value) {
            return Ok(());
        }
        let listed: Vec<String> = knob.options.iter().map(|option| option.label.clone()).collect();
        return Err(format!("{} is one of {}", knob.label, listed.join(", ")));
    }
    let (min, max) = (knob.min.unwrap_or(KnobValue(i64::MIN)), knob.max.unwrap_or(KnobValue(i64::MAX)));
    let step = knob.step.map_or(1, |step| step.0.max(1));
    if value < min || value > max || value.0.rem_euclid(step) != min.0.rem_euclid(step) {
        return Err(format!("{} is {min} to {max} in steps of {step}", knob.label));
    }
    Ok(())
}
