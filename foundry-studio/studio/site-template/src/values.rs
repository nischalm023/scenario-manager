//! Within `site-template`, `values` is an evaluation's step 3 (spec 05 section 4.3): the knob values a draft set,
//! held to their declarations, every other knob at its default, an expression default worked out in dependency
//! order, snapped up to the knob's next valid value and held to its range (D120), millimetres turned into steps of
//! the template's primary frame, and the knobs' checks evaluated into problems; it is then what every later step
//! evaluates an expression against.

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Map, Value};

use crate::evaluation::{Problem, ProblemCode};
use crate::expression::{EvalFault, Expr};
use crate::knob::{
    self, KnobCheck, KnobDecl, KnobDefault, KnobKey, KnobUnit, KnobValue, KnobValues, Outside, Severity,
};
use crate::template::{PRESENCE, Template};

/// The knob values one evaluation reads, each in the steps its expressions use.
pub(crate) struct Knobs<'t> {
    decls: &'t [KnobDecl],
    steps: FxHashMap<String, i64>,
    unit_mm: f64,
    pub values: KnobValues,
    /// The knobs nobody set whose value their default expression gave.
    pub derived: FxHashSet<KnobKey>,
    /// The derived knobs whose expression left their valid values, each held at the nearer bound.
    held: FxHashSet<KnobKey>,
}

impl<'t> Knobs<'t> {
    /// Every declared knob at the value `set` gives it, else at its default; `None`, with the problems, when a value
    /// is no knob's, off its declaration, not whole steps of the primary frame, or a default expression has no value.
    /// A default expression's value outside the knob's valid values is held at the nearer bound, with its problem.
    pub fn read(template: &'t Template<'t>, set: &KnobValues, problems: &mut Vec<Problem>) -> Option<Knobs<'t>> {
        let before = problems.len();
        for (key, _) in set.iter() {
            if !template.knobs.iter().any(|decl| decl.key == *key) {
                problems
                    .push(Problem::error(ProblemCode::ValidationFailed, format!("The template has no knob `{key}`.")));
            }
        }
        let mut knobs = Knobs {
            decls: &template.knobs,
            steps: FxHashMap::default(),
            unit_mm: template.unit_mm,
            values: KnobValues::new(),
            derived: FxHashSet::default(),
            held: FxHashSet::default(),
        };
        for decl in &template.knobs {
            let value = match (set.get(decl.key.as_str()), &decl.default) {
                (Some(value), _) => value,
                (None, KnobDefault::Plain(value)) => *value,
                (None, KnobDefault::Derived(_)) => continue,
            };
            if !decl.admits(value) {
                let why = format!("{} cannot be {}: {}", decl.label, show(value, decl.unit), allowed(decl));
                problems.push(Problem::error(ProblemCode::ValidationFailed, why).on(&decl.key));
            }
            knobs.take(decl, value, problems);
        }
        if problems.len() > before {
            return None;
        }
        // Derived defaults read set and plain values first, then each other's in dependency order.
        let Ok(order) = knob::derivation_order(&template.knobs) else {
            let why = "This template cannot be evaluated: its default expressions name one another in a cycle";
            problems.push(Problem::error(ProblemCode::TemplateInvalid, why));
            return None;
        };
        for decl in order.into_iter().map(|at| &template.knobs[at]) {
            let KnobDefault::Derived(expr) = &decl.default else { continue };
            if set.get(decl.key.as_str()).is_some() {
                continue;
            }
            match knobs.eval(expr) {
                Ok(steps) => {
                    let raw = knobs.in_unit(steps, decl.unit);
                    let (value, outside) = decl.snap_up(raw);
                    if let Some(side) = outside {
                        problems.push(out_of_range(decl, raw, side));
                        knobs.held.insert(decl.key.clone());
                    }
                    knobs.derived.insert(decl.key.clone());
                    if !knobs.take(decl, value, problems) {
                        return None;
                    }
                }
                Err(fault) => {
                    let why = format!("The default of {} has no value: {fault}", decl.label);
                    problems.push(Problem::error(fault_code(fault), why).on(&decl.key));
                    return None;
                }
            }
        }
        Some(knobs)
    }

    /// Record `decl` at `value`, in steps of the primary frame; `false`, with the problem, when it is not whole steps.
    fn take(&mut self, decl: &KnobDecl, value: KnobValue, problems: &mut Vec<Problem>) -> bool {
        self.values.set(decl.key.clone(), value);
        match to_steps(value, decl.unit, self.unit_mm) {
            Some(n) => {
                self.steps.insert(decl.key.as_str().to_owned(), n);
                true
            }
            None => {
                let why = format!(
                    "{} is {} mm, not a whole number of the template's {} mm steps",
                    decl.label, value.0, self.unit_mm
                );
                problems.push(Problem::error(ProblemCode::KnobNotWholeSteps, why).on(&decl.key));
                false
            }
        }
    }

    /// A value in steps of the primary frame as a value in `unit`: whole millimetres rounded up for a length.
    fn in_unit(&self, steps: i64, unit: KnobUnit) -> i64 {
        match unit {
            KnobUnit::Mm => (steps as f64 * self.unit_mm - 1e-6).ceil() as i64,
            KnobUnit::Count | KnobUnit::Deg => steps,
        }
    }

    pub fn eval(&self, expr: &Expr) -> Result<i64, EvalFault> {
        expr.eval(&|name| self.steps.get(name).copied().unwrap_or(0))
    }

    /// Whether an entity, a piece internal or a record is present: its `template.presence` one or more, or none.
    pub fn present(&self, components: &Map<String, Value>) -> Result<bool, EvalFault> {
        let when = components.get(PRESENCE).and_then(|presence| presence.get("when")).and_then(Value::as_str);
        match when.map(Expr::parse) {
            Some(Ok(expr)) => self.eval(&expr).map(|value| value >= 1),
            _ => Ok(true),
        }
    }

    /// Every knob's checks, each a problem under its knob while its `fails_when` is one or more. A derived knob's
    /// default is written to pass its own checks, so they report only when someone set it, or when its default
    /// falls short of them; a derived knob held at a bound has its out-of-range problem stand in for them.
    pub fn check(&self, problems: &mut Vec<Problem>) {
        for decl in self.decls.iter().filter(|decl| !self.held.contains(&decl.key)) {
            for check in &decl.checks {
                match self.eval(&check.fails_when) {
                    Ok(value) if value >= 1 => problems.push(self.failed(decl, check, value)),
                    Ok(_) => {}
                    Err(fault) => problems.push(
                        Problem::error(fault_code(fault), format!("A check of {} has no value: {fault}", decl.label))
                            .on(&decl.key),
                    ),
                }
            }
        }
    }

    fn failed(&self, decl: &KnobDecl, check: &KnobCheck, value: i64) -> Problem {
        let shown = match check.value_unit {
            KnobUnit::Mm => thousands((value as f64 * self.unit_mm).round() as i64),
            _ => thousands(value),
        };
        let mut text = check.problem.replace("{value}", &shown);
        for other in self.decls {
            if let Some(value) = self.values.get(other.key.as_str()) {
                text = text.replace(&format!("{{{}}}", other.key), &show(value, other.unit));
            }
        }
        let (severity, prefix) = match check.severity {
            Severity::Error => (Severity::Error, "Error"),
            Severity::Warning => (Severity::Warning, "Warning"),
        };
        Problem {
            severity,
            code: ProblemCode::CheckFailed,
            knob: Some(decl.key.clone()),
            reference: None,
            text: format!("{prefix}: {text}"),
        }
    }
}

fn fault_code(fault: EvalFault) -> ProblemCode {
    match fault {
        EvalFault::Overflow => ProblemCode::ExpressionOverflow,
        EvalFault::DivisionByZero => ProblemCode::DivisionByZero,
    }
}

/// A default expression whose value left the knob's valid values, under that knob.
fn out_of_range(decl: &KnobDecl, raw: i64, side: Outside) -> Problem {
    let (low, high) = decl.bounds();
    let (word, bound) = match side {
        Outside::Below => ("less than its minimum", low),
        Outside::Above => ("more than its maximum", high),
    };
    let (raw, bound) = (show(KnobValue(raw), decl.unit), show(bound, decl.unit));
    let why = format!("{}'s default comes to {raw}, {word} of {bound}, so it stays at {bound}.", decl.label);
    Problem::error(ProblemCode::DerivedOutOfRange, why).on(&decl.key)
}

/// A knob value in the steps of a primary frame `unit_mm` long: millimetres converted, refused unless whole.
fn to_steps(value: KnobValue, unit: KnobUnit, unit_mm: f64) -> Option<i64> {
    if unit != KnobUnit::Mm {
        return Some(value.0);
    }
    let steps = value.0 as f64 / unit_mm;
    let whole = steps.round();
    ((steps - whole).abs() < 1e-6).then_some(whole as i64)
}

/// A knob value as a person reads it, with its unit.
pub(crate) fn show(value: KnobValue, unit: KnobUnit) -> String {
    match unit {
        KnobUnit::Count => thousands(value.0),
        KnobUnit::Mm => format!("{} mm", thousands(value.0)),
        KnobUnit::Deg => format!("{}°", value.0),
    }
}

fn allowed(decl: &KnobDecl) -> String {
    let (low, high) = decl.bounds();
    format!("it takes {} to {}", show(low, decl.unit), show(high, decl.unit))
}

/// `8500` as `8,500`.
pub(crate) fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if n < 0 { format!("-{out}") } else { out }
}
