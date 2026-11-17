//! Human-friendly duration string parsing.

use std::time::Duration;

/// Parse a human-friendly duration string into a [`Duration`].
///
/// Accepts a non-empty sequence of `<number>[unit]` pairs. Whitespace between
/// pairs is optional, and units are case-insensitive. The number may be an
/// integer or fractional value (e.g. `1`, `1.5`, `.5`). A number with no unit
/// is interpreted as milliseconds (`"1500"` ≡ `"1500ms"`).
///
/// Recognized units:
/// - `d`  — days
/// - `h`  — hours
/// - `m`  — minutes
/// - `s`  — seconds
/// - `ms` — milliseconds (also the default when no unit is given)
///
/// Examples that all parse: `"1h"`, `"1h 30m"`, `"1h30m"`, `"1.5h"`, `"0.5h"`,
/// `".5h"`, `".5m"`, `".5d"`, `"19h"`, `"500ms"`, `"1500"`, `"1d 2h 3m 4s 500ms"`.
///
/// Repeated units are summed (`"1h 1h"` is two hours). Errors on empty input,
/// an unknown unit, a malformed number, or a negative value.
pub fn parse_human_duration(input: &str) -> Result<Duration, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("empty duration".to_string());
    }

    let mut total_ms: f64 = 0.0;
    let mut remaining = trimmed;

    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }

        let num_end = remaining.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(remaining.len());
        if num_end == 0 {
            return Err(format!("expected number in '{input}' at '{remaining}'"));
        }
        let number_str = &remaining[..num_end];
        let number: f64 = number_str.parse().map_err(|_| format!("invalid number '{number_str}' in '{input}'"))?;
        remaining = &remaining[num_end..];

        let unit_end = remaining.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(remaining.len());
        let unit_raw = &remaining[..unit_end];
        let unit_ms: f64 = match unit_raw.to_ascii_lowercase().as_str() {
            "" | "ms" => 1.0,
            "s" => 1_000.0,
            "m" => 60_000.0,
            "h" => 3_600_000.0,
            "d" => 86_400_000.0,
            other => return Err(format!("unknown unit '{other}' in '{input}'")),
        };
        total_ms += number * unit_ms;
        remaining = &remaining[unit_end..];
    }

    if !total_ms.is_finite() || total_ms < 0.0 {
        return Err(format!("invalid duration '{input}'"));
    }
    if total_ms > u64::MAX as f64 {
        return Err(format!("duration '{input}' overflows u64 milliseconds"));
    }
    Ok(Duration::from_millis(total_ms.round() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(input: &str) -> u64 {
        parse_human_duration(input).unwrap().as_millis() as u64
    }

    #[test]
    fn basic_units() {
        assert_eq!(ms("1ms"), 1);
        assert_eq!(ms("1s"), 1_000);
        assert_eq!(ms("1m"), 60_000);
        assert_eq!(ms("1h"), 3_600_000);
        assert_eq!(ms("1d"), 86_400_000);
    }

    #[test]
    fn integer_examples() {
        assert_eq!(ms("19h"), 19 * 3_600_000);
        assert_eq!(ms("500ms"), 500);
    }

    #[test]
    fn fractional() {
        assert_eq!(ms("1.5h"), 5_400_000);
        assert_eq!(ms("0.5h"), 1_800_000);
        assert_eq!(ms(".5h"), 1_800_000);
        assert_eq!(ms(".5m"), 30_000);
        assert_eq!(ms(".5d"), 43_200_000);
    }

    #[test]
    fn multiple_terms() {
        assert_eq!(ms("1h 30m"), 5_400_000);
        assert_eq!(ms("1h30m"), 5_400_000);
        assert_eq!(ms("1h500s"), 3_600_000 + 500_000);
        assert_eq!(ms("1d2h3m4s500ms"), 86_400_000 + 2 * 3_600_000 + 3 * 60_000 + 4_000 + 500);
        assert_eq!(ms("1d 2h 3m 4s 500ms"), 86_400_000 + 2 * 3_600_000 + 3 * 60_000 + 4_000 + 500);
    }

    #[test]
    fn whitespace() {
        assert_eq!(ms("  1h  "), 3_600_000);
        assert_eq!(ms("1h    30m"), 5_400_000);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(ms("1H"), 3_600_000);
        assert_eq!(ms("500MS"), 500);
        assert_eq!(ms("1H30M"), 5_400_000);
    }

    #[test]
    fn repeated_units_sum() {
        assert_eq!(ms("1h 1h"), 2 * 3_600_000);
        assert_eq!(ms("30m 30m"), 60 * 60_000);
    }

    #[test]
    fn untagged_defaults_to_ms() {
        assert_eq!(ms("1500"), 1_500);
        assert_eq!(ms("0"), 0);
        assert_eq!(ms("  42  "), 42);
        // Mixed: bare number term sums with tagged terms.
        assert_eq!(ms("1h 500"), 3_600_000 + 500);
        assert_eq!(ms("250 1s"), 250 + 1_000);
    }

    #[test]
    fn errors() {
        assert!(parse_human_duration("").is_err());
        assert!(parse_human_duration("   ").is_err());
        assert!(parse_human_duration("h").is_err()); // no number
        assert!(parse_human_duration("1x").is_err()); // unknown unit
        assert!(parse_human_duration("-1h").is_err()); // negative not allowed
        assert!(parse_human_duration("1.5.5h").is_err()); // bad number
    }
}
