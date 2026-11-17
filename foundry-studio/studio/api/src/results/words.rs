//! Within `results`, `words` is how the result routes write a quantity into a sentence: an instant or span of simtime
//! by its hours, minutes and seconds, with its milliseconds beside it where a reader needs the exact figure, its
//! thousands separated, as a person reads one; a number, rounded or exactly; and a scenario field's value in the field
//! catalogue's words (spec 07 section 3.2), so a run's inputs name each field and its unit as the Scenario panel does.

use desunits::SimTime;
use serde_json::Value;
use studio_protocol::FieldEntry;

/// "2 h 5 s", "45 min", "0 s": hours, minutes and seconds of simtime, the parts that are not zero.
pub fn sim(at: SimTime) -> String {
    let seconds = at.as_ms() / 1000;
    let parts = [(seconds / 3600, "h"), (seconds / 60 % 60, "min"), (seconds % 60, "s")];
    let said: Vec<String> =
        parts.iter().filter(|(count, _)| *count > 0).map(|(count, unit)| format!("{count} {unit}")).collect();
    match (said.is_empty(), at.as_ms()) {
        (false, _) => said.join(" "),
        (true, 0) => "0 s".to_owned(),
        (true, ms) => format!("{ms} ms"),
    }
}

/// "2 h 5 s (7,205,000 ms)": an instant of simtime as `sim` says it and exactly.
pub fn sim_exact(at: SimTime) -> String {
    format!("{} ({} ms)", sim(at), whole(at.as_ms()))
}

/// A whole number with its thousands separated: "7,205,000".
pub fn whole(value: u64) -> String {
    grouped(&value.to_string())
}

fn grouped(digits: &str) -> String {
    let mut said = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            said.push(',');
        }
        said.push(digit);
    }
    said
}

/// A number as a person reads it: a whole one with its thousands separated, any other to at most three decimals.
pub fn number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        let sign = if value < 0.0 { "-" } else { "" };
        return format!("{sign}{}", whole(value.abs() as u64));
    }
    let said = format!("{value:.3}");
    said.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// A number exactly, the shortest decimal that reads back as it, its thousands separated: "3.0001", "1,250.25".
pub fn exact(value: f64) -> String {
    let said = value.abs().to_string();
    let (digits, fraction) = said.split_once('.').map_or((said.as_str(), None), |(digits, rest)| (digits, Some(rest)));
    let sign = if value < 0.0 { "-" } else { "" };
    match fraction {
        Some(fraction) => format!("{sign}{}.{fraction}", grouped(digits)),
        None => format!("{sign}{}", grouped(digits)),
    }
}

/// A scenario field's value in the field catalogue's words: a number in the field's shown unit with that unit, a
/// closed value as its option's words up to their colon, and nothing stated as the field's absence.
pub fn field_value(field: &FieldEntry, value: &Value) -> String {
    field_words(field, value, number)
}

/// A scenario field's value as `field_value` says it, its number exactly rather than rounded.
pub fn field_value_exact(field: &FieldEntry, value: &Value) -> String {
    field_words(field, value, exact)
}

fn field_words(field: &FieldEntry, value: &Value, number: fn(f64) -> String) -> String {
    match value {
        Value::Null if field.pointer.ends_with("/to_sim_ms") => "the end of the run".to_owned(),
        Value::Null => "none".to_owned(),
        Value::Number(stated) => {
            let shown = number(stated.as_f64().unwrap_or_default() / field.per_unit);
            if field.unit.is_empty() { shown } else { format!("{shown} {}", field.unit) }
        }
        Value::String(text) => field
            .options
            .iter()
            .find(|option| option.value == *text)
            .map_or_else(|| text.clone(), |option| option.words.split(':').next().unwrap_or_default().to_lowercase()),
        other => other.to_string(),
    }
}

/// A field's value as a number in its shown unit, for a chart's axis; none for a closed value.
pub fn field_number(field: &FieldEntry, value: &Value) -> Option<f64> {
    value.as_f64().map(|stated| stated / field.per_unit)
}
