//! Within `templates`, `words` are the sentences the API writes about a person's knob changes (spec 05 section
//! 3.10): the `knob.changed` entry's text, each value in its knob's unit or by its option's label.

use studio_protocol::{EvaluationNumber, EvaluationProblem, KnobChange, KnobDeclaration, KnobUnit, KnobValue};

/// "You changed Width from 40,800 mm to 49,300 mm. Evaluation 3: no problems." (spec 05 section 3.10), a knob at
/// its default said so, since an expression default's value is the evaluation's to work out (D120).
pub fn changed_sentence(
    changes: &[KnobChange],
    declarations: &[KnobDeclaration],
    n: EvaluationNumber,
    problems: &[EvaluationProblem],
) -> String {
    let each: Vec<String> = changes
        .iter()
        .map(|change| {
            let knob = declarations.iter().find(|knob| knob.key == change.knob);
            let said = |value: Option<KnobValue>| match (knob, value) {
                (_, None) => "its default".to_owned(),
                (Some(knob), Some(value)) => knob_value(knob, value),
                (None, Some(value)) => value.to_string(),
            };
            let label = knob.map_or(change.knob.as_str(), |knob| knob.label.as_str());
            format!("{label} from {} to {}", said(change.from), said(change.to))
        })
        .collect();
    let outcome = match problems {
        [] => "no problems".to_owned(),
        [only] => format!("1 problem: {}", only.text),
        many => format!("{} problems: {}", many.len(), many[0].text),
    };
    let sentence = format!("You changed {}. Evaluation {n}: {outcome}", spoken(&each));
    if sentence.ends_with('.') { sentence } else { format!("{sentence}.") }
}

/// A knob's value in words: an option's label, or the number with its unit.
pub fn knob_value(knob: &KnobDeclaration, value: KnobValue) -> String {
    if let Some(option) = knob.options.iter().find(|option| option.value == value) {
        return option.label.clone();
    }
    let unit = match knob.unit {
        KnobUnit::Mm => " mm",
        KnobUnit::Deg => " degrees",
        KnobUnit::Count => "",
    };
    format!("{}{unit}", grouped(value.0))
}

/// 49300 as "49,300".
pub fn grouped(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (n, digit) in digits.chars().enumerate() {
        if n > 0 && (digits.len() - n).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if value < 0 { format!("-{out}") } else { out }
}

fn spoken(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
