//! Within `site-template`, a `knob` is one integer a person or the agent sets before a site is made: a
//! `template.knob` declaration, read and held to spec 05 sections 3.2 and 3.3 (one form, a valid default or a
//! default expression whose names form no cycle, a unique key, checks that parse and name declared knobs), and the
//! values a draft sets.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use rustc_hash::FxHashMap;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use studio_protocol::DeclaredId;
pub use studio_protocol::KnobValue;

use crate::expression::{Dim, Expr};

const MAX_KEY_CHARS: usize = 40;
const MAX_LABEL_CHARS: usize = 60;

/// A knob's key, `[a-z][a-z0-9_]{0,39}` and never `min` or `max`: the name expressions use for it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KnobKey(String);

impl KnobKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for KnobKey {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut chars = text.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
        let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !first_ok || !rest_ok || text.len() > MAX_KEY_CHARS {
            return Err(format!("`{text}` is not a knob key: a-z first, then a-z, 0-9 or _, at most 40 in all"));
        }
        if text == "min" || text == "max" {
            return Err(format!("`{text}` names a call, so it is never a knob key"));
        }
        Ok(Self(text.to_owned()))
    }
}

impl Borrow<str> for KnobKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Display for KnobKey {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

impl Serialize for KnobKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for KnobKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KnobUnit {
    Count,
    Mm,
    Deg,
}

impl KnobUnit {
    pub fn dim(self) -> Dim {
        match self {
            KnobUnit::Count => Dim::Count,
            KnobUnit::Mm => Dim::Length,
            KnobUnit::Deg => Dim::Angle,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnobOption {
    pub value: KnobValue,
    pub label: String,
}

/// A knob's valid values: `min + i * step` up to `max`, or a list of coded options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnobForm {
    Range { min: KnobValue, max: KnobValue, step: KnobValue },
    Options(Vec<KnobOption>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// A problem the person fixes by changing the knob that carries it: it holds while `fails_when` is one or more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnobCheck {
    pub severity: Severity,
    pub value_unit: KnobUnit,
    pub fails_when: Expr,
    /// What the person reads; `{value}` and a knob's key in braces are filled in.
    pub problem: String,
}

/// What a knob nobody set stands at (spec 05 section 3.2): one of its valid values, or the value an expression over
/// the other knobs gives at each evaluation, so an outline follows the aisles and stations it must fit (D120).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnobDefault {
    Plain(KnobValue),
    Derived(Expr),
}

/// One `template.knob` declaration, held to its rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnobDecl {
    /// The template entity that declares it.
    pub entity: DeclaredId,
    pub key: KnobKey,
    pub label: String,
    pub unit: KnobUnit,
    pub form: KnobForm,
    pub default: KnobDefault,
    pub checks: Vec<KnobCheck>,
}

#[derive(Deserialize)]
struct RawKnob {
    key: String,
    label: String,
    unit: KnobUnit,
    min: Option<i64>,
    max: Option<i64>,
    step: Option<i64>,
    options: Option<Vec<KnobOption>>,
    default: RawDefault,
    #[serde(default)]
    checks: Vec<RawCheck>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawDefault {
    Integer(i64),
    Expression(String),
}

#[derive(Deserialize)]
struct RawCheck {
    severity: Severity,
    value_unit: Option<KnobUnit>,
    fails_when: String,
    problem: String,
}

impl KnobDecl {
    /// Read one `template.knob` component, refusing what spec 05 section 3.2 refuses; checks are parsed here and
    /// their knob names resolved by [`read_all`].
    pub fn read(entity: DeclaredId, component: &Value) -> Result<KnobDecl, String> {
        let raw: RawKnob = serde_json::from_value(component.clone()).map_err(|fault| fault.to_string())?;
        let key: KnobKey = raw.key.parse()?;
        if raw.label.chars().count() > MAX_LABEL_CHARS {
            return Err(format!("the label of `{key}` is longer than {MAX_LABEL_CHARS} characters"));
        }
        let form = match (raw.min, raw.max, raw.step, raw.options) {
            (Some(min), Some(max), Some(step), None) => {
                if step < 1 || min > max {
                    return Err(format!("`{key}` needs a step of one or more and min at most max"));
                }
                KnobForm::Range { min: KnobValue(min), max: KnobValue(max), step: KnobValue(step) }
            }
            (None, None, None, Some(options)) => {
                let mut values: Vec<i64> = options.iter().map(|option| option.value.0).collect();
                values.sort_unstable();
                values.dedup();
                if values.len() < 2 || values.len() != options.len() {
                    return Err(format!("`{key}` needs two options or more, each value once"));
                }
                KnobForm::Options(options)
            }
            _ => return Err(format!("`{key}` must be exactly one form: min, max and step, or options")),
        };
        let default = match raw.default {
            RawDefault::Integer(value) => KnobDefault::Plain(KnobValue(value)),
            RawDefault::Expression(text) => {
                KnobDefault::Derived(Expr::parse(&text).map_err(|fault| format!("the default of `{key}`: {fault}"))?)
            }
        };
        let decl = KnobDecl {
            entity,
            key,
            label: raw.label,
            unit: raw.unit,
            form,
            default,
            checks: raw
                .checks
                .into_iter()
                .map(|check| {
                    Ok(KnobCheck {
                        severity: check.severity,
                        value_unit: check.value_unit.unwrap_or(KnobUnit::Count),
                        fails_when: Expr::parse(&check.fails_when).map_err(|fault| fault.to_string())?,
                        problem: check.problem,
                    })
                })
                .collect::<Result<_, String>>()?,
        };
        if let KnobDefault::Plain(value) = decl.default
            && !decl.admits(value)
        {
            return Err(format!("the default {} of `{}` is not one of its valid values", value.0, decl.key));
        }
        if decl.unit == KnobUnit::Deg && decl.stated_values().any(|value| value % 90 != 0) {
            return Err(format!("`{}` is in degrees, whose values are quarter turns", decl.key));
        }
        Ok(decl)
    }

    /// Whether `value` is one of the knob's valid values. A range's step is tested by remainders, never by
    /// `value - min`, which overflows a range wider than an `i64` holds.
    pub fn admits(&self, value: KnobValue) -> bool {
        match &self.form {
            KnobForm::Range { min, max, step } => {
                value >= *min && value <= *max && value.0.rem_euclid(step.0) == min.0.rem_euclid(step.0)
            }
            KnobForm::Options(options) => options.iter().any(|option| option.value == value),
        }
    }

    /// The least and greatest valid values; a range's greatest is `max` less how far it lies past the last step,
    /// worked out by remainders so that no difference overflows.
    pub fn bounds(&self) -> (KnobValue, KnobValue) {
        match &self.form {
            KnobForm::Range { min, max, step } => {
                let past = (max.0.rem_euclid(step.0) - min.0.rem_euclid(step.0)).rem_euclid(step.0);
                (*min, KnobValue(max.0 - past))
            }
            KnobForm::Options(options) => {
                let values = options.iter().map(|option| option.value);
                (values.clone().min().expect("two options"), values.max().expect("two options"))
            }
        }
    }

    /// The least valid value at or above `raw` (a default expression's value, in the knob's unit), and on which side
    /// `raw` lies outside the valid values when it does, the nearer bound standing in (D120). A range's snap adds to
    /// `raw` the distance to its next step, by remainders and checked: that value is at most the greatest valid value,
    /// so it fits, and were the addition ever to overflow, the greatest stands in as `derived-out-of-range`.
    pub fn snap_up(&self, raw: i64) -> (KnobValue, Option<Outside>) {
        let (low, high) = self.bounds();
        if raw < low.0 {
            return (low, Some(Outside::Below));
        }
        if raw > high.0 {
            return (high, Some(Outside::Above));
        }
        let snapped = match &self.form {
            KnobForm::Range { min, step, .. } => {
                let to_step = (min.0.rem_euclid(step.0) - raw.rem_euclid(step.0)).rem_euclid(step.0);
                raw.checked_add(to_step)
            }
            KnobForm::Options(options) => {
                options.iter().map(|option| option.value.0).filter(|value| *value >= raw).min().or(Some(high.0))
            }
        };
        match snapped {
            Some(value) => (KnobValue(value), None),
            None => (high, Some(Outside::Above)),
        }
    }

    /// The values the declaration states: an options knob's options, a range knob's bounds, step and plain default.
    fn stated_values(&self) -> impl Iterator<Item = i64> + '_ {
        let mut listed: Vec<i64> = match &self.form {
            KnobForm::Range { min, max, step } => vec![min.0, max.0, step.0],
            KnobForm::Options(options) => options.iter().map(|option| option.value.0).collect(),
        };
        if let (KnobForm::Range { .. }, KnobDefault::Plain(value)) = (&self.form, &self.default) {
            listed.push(value.0);
        }
        listed.into_iter()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outside {
    Below,
    Above,
}

/// Why a template's knob declarations cannot stand: a rule broken, or a check whose dimensions do not meet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclFault {
    Invalid(String),
    Dimension(String),
}

/// Every declaration of a template, keys unique, every check and default expression naming declared knobs in
/// dimensions that meet, and no default expression reaching itself through the others.
pub fn read_all(decls: Vec<KnobDecl>) -> Result<Vec<KnobDecl>, DeclFault> {
    let mut seen: FxHashMap<&KnobKey, DeclaredId> = FxHashMap::default();
    for decl in &decls {
        if let Some(first) = seen.insert(&decl.key, decl.entity) {
            return Err(DeclFault::Invalid(format!(
                "`{}` is the key of two knobs, {first} and {}",
                decl.key, decl.entity
            )));
        }
    }
    for decl in &decls {
        for check in &decl.checks {
            if let Some(unknown) = check.fails_when.knobs().into_iter().find(|name| !declared(&decls, name)) {
                return Err(DeclFault::Invalid(format!(
                    "a check of `{}` names `{unknown}`, which no knob declares",
                    decl.key
                )));
            }
            let dim_of = |name: &str| dim_of(&decls, name);
            check
                .fails_when
                .check_dims(&dim_of, Some(check.value_unit.dim()))
                .map_err(|fault| DeclFault::Dimension(format!("a check of `{}`: {fault}", decl.key)))?;
        }
        let KnobDefault::Derived(expr) = &decl.default else { continue };
        if let Some(unknown) = expr.knobs().into_iter().find(|name| !declared(&decls, name)) {
            return Err(DeclFault::Invalid(format!(
                "the default of `{}` names `{unknown}`, which no knob declares",
                decl.key
            )));
        }
        expr.check_dims(&|name: &str| dim_of(&decls, name), Some(decl.unit.dim()))
            .map_err(|fault| DeclFault::Dimension(format!("the default of `{}`: {fault}", decl.key)))?;
    }
    match derivation_order(&decls) {
        Ok(_) => Ok(decls),
        Err(cycle) if cycle.len() == 1 => {
            Err(DeclFault::Invalid(format!("the default of `{}` names itself", cycle[0])))
        }
        Err(cycle) => {
            let named: Vec<String> = cycle.iter().map(|key| format!("`{key}`")).collect();
            Err(DeclFault::Invalid(format!("the defaults of {} name one another in a cycle", named.join(", "))))
        }
    }
}

/// The knobs whose default is an expression, as indexes into `decls`, each after every such knob its expression
/// names, else in declaration order; `Err` names the knobs of a cycle in the order it runs.
pub(crate) fn derivation_order(decls: &[KnobDecl]) -> Result<Vec<usize>, Vec<KnobKey>> {
    fn visit(at: usize, decls: &[KnobDecl], done: &mut [bool], path: &mut Vec<usize>, out: &mut Vec<usize>) -> bool {
        if done[at] {
            return true;
        }
        if path.contains(&at) {
            path.push(at);
            return false;
        }
        let KnobDefault::Derived(expr) = &decls[at].default else {
            done[at] = true;
            return true;
        };
        path.push(at);
        for name in expr.knobs() {
            let Some(next) = decls.iter().position(|decl| decl.key.as_str() == name) else { continue };
            if !visit(next, decls, done, path, out) {
                return false;
            }
        }
        path.pop();
        done[at] = true;
        out.push(at);
        true
    }
    let (mut done, mut out) = (vec![false; decls.len()], Vec::new());
    for at in 0..decls.len() {
        let mut path = Vec::new();
        if !visit(at, decls, &mut done, &mut path, &mut out) {
            let start = path.iter().position(|seen| Some(seen) == path.last()).unwrap_or_default();
            return Err(path[start..path.len() - 1].iter().map(|at| decls[*at].key.clone()).collect());
        }
    }
    Ok(out)
}

pub(crate) fn declared(decls: &[KnobDecl], name: &str) -> bool {
    decls.iter().any(|decl| decl.key.as_str() == name)
}

pub(crate) fn dim_of(decls: &[KnobDecl], name: &str) -> Dim {
    decls.iter().find(|decl| decl.key.as_str() == name).map_or(Dim::Count, |decl| decl.unit.dim())
}

/// The knob values one evaluation reads: every declared knob, at its default unless the draft set it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KnobValues(BTreeMap<KnobKey, KnobValue>); // ordered: an evaluation echoes its knobs in key order

impl KnobValues {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: KnobKey, value: KnobValue) {
        self.0.insert(key, value);
    }

    pub fn with(mut self, key: &str, value: i64) -> Self {
        let key = key.parse().expect("a knob key written in code is one");
        self.0.insert(key, KnobValue(value));
        self
    }

    pub fn get(&self, key: &str) -> Option<KnobValue> {
        self.0.get(key).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&KnobKey, KnobValue)> {
        self.0.iter().map(|(key, value)| (key, *value))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn read(component: Value) -> Result<KnobDecl, String> {
        KnobDecl::read(DeclaredId::mint(), &component)
    }

    /// Spec 05 section 9: a declaration with both a range and options, or a default off its step, is refused.
    #[test]
    fn a_knob_has_one_form_and_a_valid_default() {
        let range = json!({ "key": "aisles", "label": "Aisles", "unit": "count", "min": 1, "max": 30, "step": 1, "default": 4 });
        assert!(read(range.clone()).is_ok());
        let mut both = range.clone();
        both["options"] = json!([{ "value": 1, "label": "one" }, { "value": 2, "label": "two" }]);
        assert!(read(both).is_err());
        let off_step = json!({ "key": "width", "label": "Width", "unit": "mm", "min": 4250, "max": 255000, "step": 850, "default": 4300 });
        assert!(read(off_step).is_err());
        let quarter = json!({ "key": "turn", "label": "Turn", "unit": "deg", "options": [{ "value": 0, "label": "0" }, { "value": 45, "label": "45" }], "default": 0 });
        assert!(read(quarter).is_err());
        assert!(
            read(json!({ "key": "max", "label": "Max", "unit": "count", "min": 0, "max": 1, "step": 1, "default": 0 }))
                .is_err()
        );
    }

    /// Spec 05 section 3.2 (D120): a default is one of the knob's valid values or an expression the grammar takes;
    /// a default that is neither refuses the knob.
    #[test]
    fn a_default_is_a_valid_value_or_an_expression() {
        let width = |default: Value| {
            read(
                json!({ "key": "width", "label": "Width", "unit": "mm", "min": 4250, "max": 255000, "step": 850, "default": default }),
            )
        };
        assert!(matches!(width(json!("aisles * 7650 + 10200")).unwrap().default, KnobDefault::Derived(_)));
        assert_eq!(width(json!(40800)).unwrap().default, KnobDefault::Plain(KnobValue(40800)));
        for refused in [json!("aisles +"), json!("Aisles * 2"), json!(40800.5), json!(40000)] {
            assert!(width(refused.clone()).is_err(), "{refused}");
        }
    }

    /// Spec 05 section 3.2 (D120): a default expression's value snaps up to the knob's next valid value; outside the
    /// valid values the nearer bound stands in, with the side it left by.
    #[test]
    fn a_derived_value_snaps_up_to_a_valid_value_and_holds_to_the_bounds() {
        let width = read(json!({ "key": "width", "label": "Width", "unit": "mm", "min": 4250, "max": 255000, "step": 850, "default": 40800 })).unwrap();
        assert_eq!(width.snap_up(40800), (KnobValue(40800), None));
        assert_eq!(width.snap_up(40801), (KnobValue(41650), None));
        assert_eq!(width.snap_up(4249), (KnobValue(4250), Some(Outside::Below)));
        assert_eq!(width.snap_up(255001), (KnobValue(255000), Some(Outside::Above)));
        let turns = read(json!({ "key": "turn", "label": "Turn", "unit": "deg", "options": [{ "value": 0, "label": "none" }, { "value": 180, "label": "half" }], "default": 0 })).unwrap();
        assert_eq!(turns.snap_up(90), (KnobValue(180), None));
        assert_eq!(turns.snap_up(270), (KnobValue(180), Some(Outside::Above)));
    }

    /// Spec 05 section 3.3's overflow rule for a snap (design/reviews-impl/astra-07c.md, the snap-up finding): a count
    /// from 0 to `i64::MAX` in steps of 3 has `i64::MAX - 1` as its greatest valid value, and a derived value at or
    /// just below it snaps to it exactly, where `raw - min + step - 1` overflowed; one past it is
    /// `derived-out-of-range`'s side. A range wider than an `i64` holds (from `i64::MIN`) admits and snaps without a
    /// difference overflowing either.
    #[test]
    fn a_derived_value_at_the_top_of_the_integers_snaps_without_overflow() {
        let top = read(json!({ "key": "far", "label": "Far", "unit": "count", "min": 0, "max": i64::MAX, "step": 3, "default": 0 })).unwrap();
        let greatest = KnobValue(i64::MAX - 1);
        assert_eq!(top.bounds(), (KnobValue(0), greatest));
        assert_eq!(top.snap_up(i64::MAX - 1), (greatest, None));
        assert_eq!(top.snap_up(i64::MAX - 2), (greatest, None));
        assert_eq!(top.snap_up(i64::MAX - 4), (KnobValue(i64::MAX - 4), None));
        assert_eq!(top.snap_up(i64::MAX), (greatest, Some(Outside::Above)));
        assert!(top.admits(greatest) && !top.admits(KnobValue(i64::MAX)));
        let wide = read(json!({ "key": "wide", "label": "Wide", "unit": "count", "min": i64::MIN, "max": i64::MAX, "step": 2, "default": 0 })).unwrap();
        assert_eq!(wide.bounds(), (KnobValue(i64::MIN), KnobValue(i64::MAX - 1)));
        assert_eq!(wide.snap_up(-1), (KnobValue(0), None));
        assert!(wide.admits(KnobValue(i64::MIN)) && !wide.admits(KnobValue(1)));
    }

    /// Spec 05 section 3.2 (D120): default expressions name declared knobs only and are worked out in dependency
    /// order, whatever the order of declaration; one reaching itself through the others is refused.
    #[test]
    fn default_expressions_name_declared_knobs_in_an_order_without_a_cycle() {
        let decl = |key: &str, default: Value| {
            read(
                json!({ "key": key, "label": key, "unit": "count", "min": 0, "max": 9, "step": 1, "default": default }),
            )
            .unwrap()
        };
        let ordered = read_all(vec![decl("a", json!("b + 1")), decl("b", json!("c")), decl("c", json!(2))]).unwrap();
        assert_eq!(derivation_order(&ordered), Ok(vec![1, 0]));
        let cycle = Err(DeclFault::Invalid("the defaults of `a`, `b` name one another in a cycle".into()));
        assert_eq!(read_all(vec![decl("c", json!(1)), decl("a", json!("b + c")), decl("b", json!("a"))]), cycle);
        let itself = Err(DeclFault::Invalid("the default of `a` names itself".into()));
        assert_eq!(read_all(vec![decl("a", json!("a + 1"))]), itself);
        assert!(matches!(read_all(vec![decl("a", json!("d"))]), Err(DeclFault::Invalid(_))));
    }

    /// Spec 05 section 9: a duplicate key is refused, the fault naming the knob declared first and the one after it.
    #[test]
    fn keys_are_unique_and_checks_name_declared_knobs() {
        let decl = |key: &str, checks: Value| {
            read(json!({ "key": key, "label": key, "unit": "count", "min": 0, "max": 9, "step": 1, "default": 1, "checks": checks })).unwrap()
        };
        let check = |fails_when: &str| json!([{ "severity": "error", "fails_when": fails_when, "problem": "no" }]);
        let (first, second) = (decl("a", json!([])), decl("a", json!([])));
        let named = format!("`a` is the key of two knobs, {} and {}", first.entity, second.entity);
        assert_eq!(read_all(vec![decl("b", json!([])), first, second]), Err(DeclFault::Invalid(named)));
        assert!(matches!(read_all(vec![decl("a", check("b - 1"))]), Err(DeclFault::Invalid(_))));
        assert!(read_all(vec![decl("a", check("a - 1")), decl("b", json!([]))]).is_ok());
    }
}
