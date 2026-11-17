//! Spec 03 section 8, testable statement 1 (the grammar): every example of section 3.1's grammar parses and prints
//! back unchanged, and each refused form is refused, read from the golden-vector file every parser shares.

use std::fmt::{Debug, Display};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use studio_protocol::{
    BranchName, EntityRef, GrammarError, Key, KinPins, Kinship, Location, ObjectKey, Pin, ProblemType, Ref, ScopePath,
};

#[derive(Deserialize)]
struct Vectors {
    parse: Vec<Parse>,
    refuse: Vec<Refuse>,
    pin_rule: Vec<PinRule>,
    kin_beside: Vec<KinBeside>,
}

#[derive(Deserialize)]
struct Parse {
    form: String,
    text: String,
    canonical: Option<String>,
}

#[derive(Deserialize)]
struct Refuse {
    form: String,
    text: String,
}

#[derive(Deserialize)]
struct PinRule {
    #[serde(rename = "ref")]
    reference: String,
    kinship: String,
    refused: Option<ProblemType>,
    keeps: Option<String>,
}

#[derive(Deserialize)]
struct KinBeside {
    kin: String,
    location: String,
    site: String,
    refused: bool,
}

fn vectors() -> Vectors {
    serde_json::from_str(include_str!("grammar-vectors.json")).expect("the vector file is well formed")
}

/// Read `text` as `T` and print it back; the same text must come back through serde.
fn reprint<T>(text: &str) -> Result<String, GrammarError>
where
    T: FromStr<Err = GrammarError> + Display + Serialize + for<'de> Deserialize<'de> + PartialEq + Debug,
{
    let value: T = text.parse()?;
    let json = serde_json::to_value(&value).expect("a grammar value serializes");
    assert_eq!(json, Value::String(value.to_string()), "serde writes the canonical text of {text}");
    assert_eq!(serde_json::from_value::<T>(json).expect("serde reads back what it wrote"), value);
    Ok(value.to_string())
}

fn reprint_as(form: &str, text: &str) -> Result<String, GrammarError> {
    match form {
        "scope-path" => reprint::<ScopePath>(text),
        "branch" => reprint::<BranchName>(text),
        "pin" => reprint::<Pin>(text),
        "location" => reprint::<Location>(text),
        "kin-pins" => reprint::<KinPins>(text),
        "object" => reprint::<ObjectKey>(text),
        "entity-ref" => reprint::<EntityRef>(text),
        "key" => reprint::<Key>(text),
        "ref" => reprint::<Ref>(text),
        other => panic!("the vector file names a form `{other}` this test does not know"),
    }
}

/// Spec 03 section 8, statement 1: every example of section 3.1's grammar, `submission:{ULID}`, a field key and the
/// `kin` pins `flow@2,fleet@1` parse and print back unchanged.
#[test]
fn every_example_parses_and_prints_its_canonical_text() {
    for vector in vectors().parse {
        let printed = reprint_as(&vector.form, &vector.text)
            .unwrap_or_else(|fault| panic!("{} `{}` was refused: {fault}", vector.form, vector.text));
        assert_eq!(printed, vector.canonical.unwrap_or(vector.text), "{}", vector.form);
    }
}

/// Spec 03 section 8, statement 1: `@head` in a location, `@01`, a slug `-x` or `x-`, an all-digit branch, a branch
/// named `head`, an upper-case UUID, a second `@`, and a `kin` pin with a branch or a tag are refused, each as
/// `ref-malformed` naming the whole string.
#[test]
fn every_refused_form_is_refused_naming_the_string() {
    for vector in vectors().refuse {
        match reprint_as(&vector.form, &vector.text) {
            Ok(printed) => panic!("{} `{}` was read as `{printed}`", vector.form, vector.text),
            Err(fault) => {
                assert_eq!(fault.input(), vector.text, "the refusal names the whole string: {fault}");
                assert_eq!(fault.problem_type(), ProblemType::RefMalformed);
            }
        }
    }
}

/// Spec 03 section 8, statement 1: a lateral reference pinned to a branch, and an `@head` reference to another site's
/// entity, are refused; section 3.1's rules 4 and 5 keep or refuse every other pin, kin and lateral.
#[test]
fn rules_four_and_five_keep_or_refuse_each_reference() {
    for vector in vectors().pin_rule {
        let kinship = match vector.kinship.as_str() {
            "kin" => Kinship::Kin,
            "lateral" => Kinship::Lateral,
            other => panic!("the vector file names a kinship `{other}`"),
        };
        let reference: EntityRef = vector.reference.parse().expect("a pin-rule vector is a reference");
        match (reference.under_pin_rule(kinship), vector.refused, vector.keeps) {
            (Ok(kept), None, Some(keeps)) => assert_eq!(kept.to_string(), keeps),
            (Err(fault), Some(refused), None) => {
                assert_eq!(fault.breach.problem_type(), refused, "{fault}");
                assert!(fault.to_string().contains(&vector.reference), "the refusal names the reference: {fault}");
            }
            (outcome, refused, keeps) => {
                panic!("`{}` ({kinship:?}) came out {outcome:?}; expected {refused:?} or {keeps:?}", vector.reference)
            }
        }
    }
}

/// Spec 03 section 8, statement 1: a `kin` pin naming the shown scope, or beside a branch location, is refused; and,
/// by section 3.1's scope grammar, so is one that would name a scope past eight slugs under the site.
#[test]
fn kin_pins_stand_only_beside_a_changeset_location_and_never_name_it() {
    for vector in vectors().kin_beside {
        let kin: KinPins = vector.kin.parse().expect("a kin vector's pins parse");
        let location: Location = vector.location.parse().expect("a kin vector's location parses");
        let site: ScopePath = vector.site.parse().expect("a kin vector's site parses");
        let outcome = kin.check_beside(&location, &site);
        assert_eq!(outcome.is_err(), vector.refused, "`{}` beside `{}`: {outcome:?}", vector.kin, vector.location);
    }
}
