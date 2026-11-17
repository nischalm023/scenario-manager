//! The grid counters the heat maps are drawn from: how often each cell was
//! visited, and whatever else the producer counts per cell.
//!
//! The counters come in named layers, because the producer decides what it
//! counts and the consumer discovers the layers from the recording rather
//! than from a list compiled into it. A keyframe carries every layer's counts
//! at its instant; a delta carries what one slice added, and only for the
//! cells that slice touched.
//!
//! Both are state rather than events — a counter per cell, with no order of
//! its own — so the encoder writes layers by name and counts by cell id.

use desgraph::types::CellId;
use serde::ser::SerializeTuple;
use serde::{Deserialize, Serialize, Serializer};

use crate::container::payload::Payload;
use crate::frames::in_id_order;

/// One layer's counts at a keyframe's instant, sparse over the grid.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct HeatLayer {
    /// What the layer counts, in the producer's own words — for instance
    /// `nav_visits_htm`.
    pub name: String,
    pub counts: Vec<(CellId, u64)>,
}

/// One layer's increments over one slice.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct HeatLayerDelta {
    pub name: String,
    pub increments: Vec<(CellId, u64)>,
}

/// Every layer's counters at this frame's instant.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct HeatKeyframe {
    pub layers: Vec<HeatLayer>,
}

/// What one slice added to each layer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct HeatDelta {
    pub layers: Vec<HeatLayerDelta>,
}

fn layer<S: Serializer>(serializer: S, name: &str, counts: &[(CellId, u64)]) -> Result<S::Ok, S::Error> {
    let mut row = serializer.serialize_tuple(2)?;
    row.serialize_element(name)?;
    row.serialize_element(&in_id_order(counts, |(cell, _)| *cell))?;
    row.end()
}

impl Serialize for HeatLayer {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        layer(serializer, &self.name, &self.counts)
    }
}

impl Serialize for HeatLayerDelta {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        layer(serializer, &self.name, &self.increments)
    }
}

/// The layers of one frame, written by name.
fn layers<S: Serializer, T>(serializer: S, layers: &[T], name: fn(&T) -> &str) -> Result<S::Ok, S::Error>
where
    T: Serialize,
{
    let mut by_name: Vec<&T> = layers.iter().collect();
    by_name.sort_by(|left, right| name(left).cmp(name(right)));
    let mut row = serializer.serialize_tuple(1)?;
    row.serialize_element(&by_name)?;
    row.end()
}

impl Serialize for HeatKeyframe {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        layers(serializer, &self.layers, |layer| &layer.name)
    }
}

impl Serialize for HeatDelta {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        layers(serializer, &self.layers, |layer| &layer.name)
    }
}

impl Payload for HeatKeyframe {
    const NAME: &'static str = "heat keyframe";
}

impl Payload for HeatDelta {
    const NAME: &'static str = "heat delta";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heat_delta_is_layers_of_sparse_increments_written_by_name_and_by_cell() {
        let delta = HeatDelta {
            layers: vec![
                HeatLayerDelta { name: "nav_visits_vtm".to_owned(), increments: vec![(CellId(9), 1)] },
                HeatLayerDelta { name: "nav_visits_htm".to_owned(), increments: vec![(CellId(9), 1), (CellId(4), 2)] },
            ],
        };
        let bytes = delta.to_bytes().expect("encodes");
        assert_eq!(bytes[0], 0x91, "one element, the layers");
        assert_eq!(bytes[1], 0x92, "two layers");
        let read = HeatDelta::from_bytes(&bytes, 0).expect("decodes");
        assert_eq!(read.layers[0].name, "nav_visits_htm", "layers come back by name");
        assert_eq!(read.layers[0].increments, vec![(CellId(4), 2), (CellId(9), 1)], "counts come back by cell");
    }
}
