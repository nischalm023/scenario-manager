//! Within `flowgraph`, a `quantity` is one of the flow components' numbers in its own type (spec 06 section 3.1),
//! held as it is read to the bound its registered schema states (design/vocabulary): a fill pace and an admission
//! factor above zero; a fairness bound, a layer, a cap, a tote's units, a backlog and a step whole numbers from one.
//! A value outside its bound leaves its component unreadable (`schema-unknown`, its sentence naming the field), so
//! neither a site nor a sweep's patch reaches a plan with one. Each travels as the bare number it was written as.

use serde::{Deserialize, Serialize};

/// A rate per hour of simtime, such as a station's fill pace or its all-in rate: above zero.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct PerHour(f64);

impl PerHour {
    pub fn new(per_hour: f64) -> Option<Self> {
        Self::try_from(per_hour).ok()
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for PerHour {
    type Error = String;

    fn try_from(per_hour: f64) -> Result<Self, String> {
        above_zero("a rate per hour", per_hour).map(Self)
    }
}

impl From<PerHour> for f64 {
    fn from(rate: PerHour) -> f64 {
        rate.0
    }
}

/// An intake's admission factor (`intake[].admission_factor`): each member admits at most `ceil(factor x F_max / P)
/// + 2` carriers in flight, so a factor of zero or below would admit none.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct AdmissionFactor(f64);

impl AdmissionFactor {
    pub fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for AdmissionFactor {
    type Error = String;

    fn try_from(factor: f64) -> Result<Self, String> {
        above_zero("admission_factor", factor).map(Self)
    }
}

impl From<AdmissionFactor> for f64 {
    fn from(factor: AdmissionFactor) -> f64 {
        factor.0
    }
}

fn above_zero(what: &str, value: f64) -> Result<f64, String> {
    if value.is_finite() && value > 0.0 { Ok(value) } else { Err(format!("{what} is above zero, and {value} is not")) }
}

/// A whole number from one that a component's field counts in, named for that field: read from any JSON integer so
/// a zero or a negative is refused in the field's own words.
macro_rules! whole_from_one {
    ($(#[$doc:meta])* $name:ident, $field:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(try_from = "i64", into = "u32")]
        pub struct $name(u32);

        impl $name {
            pub const fn get(self) -> u32 {
                self.0
            }
        }

        impl TryFrom<i64> for $name {
            type Error = String;

            fn try_from(count: i64) -> Result<Self, String> {
                u32::try_from(count)
                    .ok()
                    .filter(|&count| count >= 1)
                    .map(Self)
                    .ok_or_else(|| format!("{} is a whole number from 1, and {count} is not", $field))
            }
        }

        impl From<$name> for u32 {
            fn from(count: $name) -> u32 {
                count.0
            }
        }
    };
}

whole_from_one!(
    /// The most consecutive picks from one order while another bin's head is ready (`work.fairness`).
    Fairness,
    "fairness"
);
whole_from_one!(
    /// How many totes one layer holds under layer sequencing (`work.layer_size`).
    LayerSize,
    "layer_size"
);
whole_from_one!(
    /// The most orders a pick conveyance holds open at once (`work.max_open_orders`).
    OpenOrderCap,
    "max_open_orders"
);
whole_from_one!(
    /// The pinned cap on carriers in flight to each member (`intake[].max_inflight`).
    InFlightCap,
    "max_inflight"
);
whole_from_one!(
    /// How many units fill one inbound tote (`induction.units_per_tote`).
    UnitsPerTote,
    "units_per_tote"
);
whole_from_one!(
    /// A return rule's backlog (`min_backlog`), kept so that documents naming it still load: no run reads it.
    Backlog,
    "min_backlog"
);
whole_from_one!(
    /// A segment's place in its edge's chain (`step`).
    Step,
    "step"
);

impl UnitsPerTote {
    /// The schema's default: one unit per inbound tote.
    pub const ONE: Self = Self(1);
}

#[cfg(test)]
mod tests {
    use serde_json::{from_value, json, to_value};

    use super::*;

    /// Each bound refuses its edge in its field's words, and a value inside it travels as the number it was.
    #[test]
    fn each_quantity_refuses_what_its_schema_refuses_and_keeps_its_number() {
        let refused = |value| from_value::<AdmissionFactor>(json!(value)).unwrap_err().to_string();
        assert_eq!(refused(0.0), "admission_factor is above zero, and 0 is not");
        assert_eq!(refused(-1.5), "admission_factor is above zero, and -1.5 is not");
        let backlog = |value| from_value::<Backlog>(json!(value)).map_err(|fault| fault.to_string());
        assert_eq!(backlog(0), Err("min_backlog is a whole number from 1, and 0 is not".to_owned()));
        assert_eq!(backlog(-2), Err("min_backlog is a whole number from 1, and -2 is not".to_owned()));
        assert!(from_value::<Step>(json!(0)).is_err() && from_value::<PerHour>(json!(0)).is_err());
        assert!(from_value::<Fairness>(json!(4_294_967_296_i64)).is_err(), "beyond a u32 is no count");
        assert_eq!(to_value(from_value::<AdmissionFactor>(json!(4.0)).unwrap()).unwrap(), json!(4.0));
        assert_eq!(to_value(backlog(1).unwrap()).unwrap(), json!(1));
        assert_eq!(PerHour::new(f64::INFINITY), None);
    }
}
