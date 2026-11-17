//! A descriptor stated after the run has begun, and the running list of every
//! descriptor a reader has been given.
//!
//! A run does not always know at its start what it will record: a metric
//! appears when the thing it measures first exists, and a table when the
//! first row of it is written. The scenario block states the descriptors
//! known then, and a declare frame states one more, stamped with the start of
//! the slice it precedes. Declarations only ever append, so a slot in a
//! scalars array and a table's id never move once a run has stated them.

use std::fmt;

use serde::de::{Error as DeError, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::container::payload::Payload;
use crate::frames::byte_enum;
use crate::frames::scalars::MetricDescriptor;
use crate::frames::scenario::Scenario;
use crate::frames::tables::{TableDescriptor, TableId};

byte_enum! {
    /// Which kind of descriptor a declare frame carries.
    DeclareKind as "declare kind" {
        Metric = 1,
        Table = 2,
    }
}

/// One descriptor appearing after the scenario block: two positions on the
/// wire, the kind byte and the descriptor the kind names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Declare {
    Metric(MetricDescriptor),
    Table(TableDescriptor),
}

impl Declare {
    pub fn kind(&self) -> DeclareKind {
        match self {
            Self::Metric(_) => DeclareKind::Metric,
            Self::Table(_) => DeclareKind::Table,
        }
    }
}

impl Serialize for Declare {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut row = serializer.serialize_tuple(2)?;
        row.serialize_element(&self.kind())?;
        match self {
            Self::Metric(descriptor) => row.serialize_element(descriptor)?,
            Self::Table(descriptor) => row.serialize_element(descriptor)?,
        }
        row.end()
    }
}

struct DeclareForm;

impl<'de> Visitor<'de> for DeclareForm {
    type Value = Declare;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a declare kind byte and the descriptor it names")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut row: A) -> Result<Declare, A::Error> {
        let kind: DeclareKind = row.next_element()?.ok_or_else(|| DeError::invalid_length(0, &self))?;
        let missing = || DeError::invalid_length(1, &self);
        Ok(match kind {
            DeclareKind::Metric => Declare::Metric(row.next_element::<MetricDescriptor>()?.ok_or_else(missing)?),
            DeclareKind::Table => Declare::Table(row.next_element::<TableDescriptor>()?.ok_or_else(missing)?),
        })
    }
}

impl<'de> Deserialize<'de> for Declare {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_tuple(2, DeclareForm)
    }
}

impl Payload for Declare {
    const NAME: &'static str = "declare";
}

/// Every descriptor a reader has been given, in the order they were stated:
/// what each slot of a scalars array counts, and what each table's rows are.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Descriptors {
    pub metrics: Vec<MetricDescriptor>,
    pub tables: Vec<TableDescriptor>,
}

impl Descriptors {
    /// The descriptors the run began with.
    pub fn from_scenario(scenario: &Scenario) -> Self {
        Self { metrics: scenario.metrics.clone(), tables: scenario.tables.clone() }
    }

    /// Take in one declaration. It appends, because a slot that moved would
    /// change what every frame written before it meant.
    pub fn push(&mut self, declared: &Declare) {
        match declared {
            Declare::Metric(descriptor) => self.metrics.push(descriptor.clone()),
            Declare::Table(descriptor) => self.tables.push(descriptor.clone()),
        }
    }

    /// Which slot of a scalars array a metric's values sit in.
    pub fn metric_slot(&self, name: &str) -> Option<usize> {
        self.metrics.iter().position(|metric| metric.name == name)
    }

    pub fn table(&self, id: TableId) -> Option<&TableDescriptor> {
        self.tables.iter().find(|table| table.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::scalars::{AggregationClass, MetricSemantics, MetricUnit};

    fn metric(name: &str) -> MetricDescriptor {
        MetricDescriptor {
            name: name.to_owned(),
            label: name.to_owned(),
            description: String::new(),
            semantics: MetricSemantics {
                unit: MetricUnit::Count,
                denominator_description: None,
                aggregation: AggregationClass::Additive,
            },
        }
    }

    #[test]
    fn a_declare_is_a_kind_byte_and_the_descriptor_it_names() {
        let declared = Declare::Metric(metric("picks_total"));
        let bytes = declared.to_bytes().expect("encodes");
        assert_eq!(bytes[0], 0x92, "two positions");
        assert_eq!(bytes[1], DeclareKind::Metric.as_byte());
        assert_eq!(Declare::from_bytes(&bytes, 0).expect("decodes"), declared);

        let table = Declare::Table(TableDescriptor {
            id: TableId(3),
            name: "fulfillment.picks".to_owned(),
            columns: Vec::new(),
            instant_column: None,
        });
        let bytes = table.to_bytes().expect("encodes");
        assert_eq!(bytes[1], DeclareKind::Table.as_byte(), "the kind byte says which descriptor follows");
        assert_eq!(Declare::from_bytes(&bytes, 0).expect("decodes"), table);
    }

    #[test]
    fn a_declaration_appends_so_the_slots_a_run_has_stated_never_move() {
        let mut descriptors = Descriptors { metrics: vec![metric("a"), metric("b")], tables: Vec::new() };
        descriptors.push(&Declare::Metric(metric("c")));
        assert_eq!(descriptors.metric_slot("a"), Some(0));
        assert_eq!(descriptors.metric_slot("c"), Some(2));
        assert_eq!(descriptors.metric_slot("d"), None);
    }
}
