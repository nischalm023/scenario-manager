//! Within the Map Creator adapter, `source_index` is **the dump's own join tables**:
//! the four legacy keys one Map Creator record uses to name another, each
//! resolved to the identity the import gave that record.
//!
//! It exists because the export names things four ways and the site names
//! them one way. `charger.json` points at a cell by the barcode on the concrete,
//! `block.json` by a raw grid pair, `tote_storage.json` by a bracketed grid
//! string, and `pps.json` by a station id that is an integer in one dialect and a
//! string in the other. Every one of those becomes an entity identity on the way
//! in (SPEC §2 keeps a display string out of reference position), and this is
//! where the lookup happens, once, after every record has an id.
//!
//! A key that names nothing is not corrected and not dropped. The caller records
//! it as a dangling source reference and carries the legacy string verbatim, so
//! the export writes back what the dump said — twenty-four of one site's hundred
//! and eighty obstacle-detection exclusions name a barcode with no cell, and a
//! converter that quietly lost them would be hiding a fact about the site.
//!
//! Every reference a record makes resolves here, through [`SourceIndex::cell`],
//! [`SourceIndex::joined`] or [`SourceIndex::sector`], and only when it is
//! spelled exactly as the export writes the named record's key back; anything
//! else names nothing, and the caller keeps it as the file wrote it, so it comes
//! back as it went in rather than respelled.

use crate::identity::scalar;
use crate::format::source::{bracket_pair, coordinate_key, raw_pair};
use crate::records::{Join, KeyText, Spelling};
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;
use uuid::Uuid;
use worldcanonical::to_canonical_string;

/// What one cell reference names in this dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellNamed {
    /// One cell.
    One(Uuid),
    /// No cell: the reference is kept as the dump wrote it.
    Nothing,
    /// A barcode more than one cell carries, so which cell it means is a
    /// guess; the barcode is here for the refusal to name.
    Shared(String),
}

/// Every legacy key of one dump, resolved to the identity its record got.
#[derive(Debug, Default)]
pub struct SourceIndex {
    /// A cell's own `coordinate` string, as `map.json` spells it -> the cell's
    /// identity. A raw pair is spelled `"[x,y]"` before lookup.
    by_coordinate: FxHashMap<String, Uuid>,
    /// The barcode on the concrete -> the cell's identity, for a barcode one
    /// cell carries.
    by_barcode: FxHashMap<String, Uuid>,
    /// The barcodes more than one cell carries. Nothing is wrong with them
    /// until a reference names one.
    shared: FxHashSet<String>,
    /// A cell's identity -> its measured position in millimetres, so a pose
    /// is read off the very cell a reference resolved to.
    positions: FxHashMap<Uuid, [f64; 2]>,
    /// A cell's identity -> its own `coordinate` string, so a fact rebuilt from
    /// the cell a reference resolved to is rebuilt from that very cell.
    coordinates: FxHashMap<Uuid, String>,
    /// A `pps_id` in string form -> the station's identity.
    by_station: FxHashMap<String, Uuid>,
    /// A `block_id` in string form -> the block's identity.
    by_block: FxHashMap<String, Uuid>,
    /// A `zone_id` -> the zone's identity.
    by_zone: FxHashMap<String, Uuid>,
    /// A `location_profile_id` -> the profile's identity.
    by_profile: FxHashMap<String, Uuid>,
    /// A sector key, `"undefined"` included -> the sector's identity and the
    /// JSON value the cells spell its key as.
    by_sector: FxHashMap<String, (Uuid, Value)>,
    /// A `seat_name` under a station -> the seat's identity.
    by_seat: FxHashMap<(String, String), Uuid>,
    /// A `bin_id` under a station -> the bin's identity.
    by_bin: FxHashMap<(String, String), Uuid>,
    /// A bin type id -> its identity.
    by_bin_type: FxHashMap<String, Uuid>,
    /// A `conveyor_id` in string form -> the conveyor's identity.
    by_conveyor: FxHashMap<String, Uuid>,
}

impl SourceIndex {
    pub fn add_cell(&mut self, coordinate: &str, barcode: &str, id: Uuid, world: [f64; 2]) {
        self.by_coordinate.insert(coordinate.to_string(), id);
        if self.by_barcode.insert(barcode.to_string(), id).is_some() {
            self.shared.insert(barcode.to_string());
        }
        self.positions.insert(id, world);
        self.coordinates.insert(id, coordinate.to_string());
    }

    /// The measured position of the cell one reference resolved to.
    pub fn position(&self, cell: Uuid) -> Option<[f64; 2]> {
        self.positions.get(&cell).copied()
    }

    /// The `coordinate` string of the cell one reference resolved to.
    pub fn coordinate(&self, cell: Uuid) -> Option<&str> {
        self.coordinates.get(&cell).map(String::as_str)
    }

    pub fn add_station(&mut self, pps_id: &str, id: Uuid) {
        self.by_station.insert(pps_id.to_string(), id);
    }

    pub fn add_block(&mut self, block_id: &str, id: Uuid) {
        self.by_block.insert(block_id.to_string(), id);
    }

    pub fn add_zone(&mut self, zone_id: &str, id: Uuid) {
        self.by_zone.insert(zone_id.to_string(), id);
    }

    pub fn add_sector(&mut self, key: &str, id: Uuid, spelled: &Value) {
        self.by_sector.insert(key.to_string(), (id, spelled.clone()));
    }

    /// The sector one value names, spelled as the cells spell its key, which
    /// is how the export writes a reference to it back.
    pub fn sector(&self, value: &Value) -> Option<Uuid> {
        let (id, spelled) = self.by_sector.get(&scalar(value))?;
        (to_canonical_string(spelled) == to_canonical_string(value)).then_some(*id)
    }

    pub fn add_profile(&mut self, profile_id: &str, id: Uuid) {
        self.by_profile.insert(profile_id.to_string(), id);
    }

    pub fn add_seat(&mut self, pps_id: &str, seat: &str, id: Uuid) {
        self.by_seat.insert((pps_id.to_string(), seat.to_string()), id);
    }

    pub fn add_bin(&mut self, pps_id: &str, bin: &str, id: Uuid) {
        self.by_bin.insert((pps_id.to_string(), bin.to_string()), id);
    }

    pub fn add_bin_type(&mut self, type_id: &str, id: Uuid) {
        self.by_bin_type.insert(type_id.to_string(), id);
    }

    pub fn add_conveyor(&mut self, conveyor_id: &str, id: Uuid) {
        self.by_conveyor.insert(conveyor_id.to_string(), id);
    }

    /// The cell one value names, read in the spelling its field uses: the one
    /// function every reference to a cell, and every pose read off one,
    /// resolves through.
    pub fn cell(&self, value: &Value, spelling: Spelling) -> CellNamed {
        let found = match spelling {
            Spelling::Barcode => {
                let Some(barcode) = value.as_str() else { return CellNamed::Nothing };
                if self.shared.contains(barcode) {
                    return CellNamed::Shared(barcode.to_string());
                }
                self.by_barcode.get(barcode).copied()
            }
            // A coordinate string names the cell whose own coordinate it is,
            // spelled exactly so: the export writes that cell's coordinate back,
            // so a respelling of it would not come back.
            Spelling::Bracketed => value.as_str().and_then(|text| self.by_coordinate.get(text)).copied(),
            Spelling::RawPair => coordinate_of(value, spelling).and_then(|key| self.by_coordinate.get(&key)).copied(),
        };
        found.map_or(CellNamed::Nothing, CellNamed::One)
    }

    /// The cell a `"[x,y]"` coordinate string names.
    pub fn cell_by_coordinate(&self, coordinate: &str) -> Option<Uuid> {
        self.by_coordinate.get(coordinate).copied()
    }

    pub fn station(&self, pps_id: &str) -> Option<Uuid> {
        self.by_station.get(pps_id).copied()
    }

    pub fn block(&self, block_id: &str) -> Option<Uuid> {
        self.by_block.get(block_id).copied()
    }

    pub fn zone(&self, zone_id: &str) -> Option<Uuid> {
        self.by_zone.get(zone_id).copied()
    }

    pub fn profile(&self, profile_id: &str) -> Option<Uuid> {
        self.by_profile.get(profile_id).copied()
    }

    pub fn seat(&self, pps_id: &str, seat: &str) -> Option<Uuid> {
        self.by_seat.get(&(pps_id.to_string(), seat.to_string())).copied()
    }

    pub fn bin(&self, pps_id: &str, bin: &str) -> Option<Uuid> {
        self.by_bin.get(&(pps_id.to_string(), bin.to_string())).copied()
    }

    pub fn bin_type(&self, type_id: &str) -> Option<Uuid> {
        self.by_bin_type.get(type_id).copied()
    }

    /// The record one value names by that record's own key, or `None` when it
    /// names nothing or is not spelled in `text`, the JSON kind its field's file
    /// fixes and the export writes the key back in. A seat and a bin are named
    /// among the parts of `station`, the naming record's own station.
    pub fn joined(&self, join: Join, text: KeyText, value: &Value, station: &str) -> Option<Uuid> {
        let spelled = match text {
            KeyText::Text => value.is_string(),
            KeyText::Number => value.is_number(),
        };
        if !spelled {
            return None;
        }
        let key = scalar(value);
        match join {
            Join::Station => self.station(&key),
            Join::Zone => self.zone(&key),
            Join::Profile => self.profile(&key),
            Join::Seat => self.seat(station, &key),
            Join::Bin => self.bin(station, &key),
            Join::BinType => self.bin_type(&key),
            Join::Conveyor => self.conveyor(&key),
            Join::Block => self.block(&key),
        }
    }

    pub fn conveyor(&self, conveyor_id: &str) -> Option<Uuid> {
        self.by_conveyor.get(conveyor_id).copied()
    }
}

/// The canonical `"[x,y]"` key one value spells, whichever of the two coordinate
/// spellings it arrived in.
pub fn coordinate_of(value: &Value, spelling: Spelling) -> Option<String> {
    match spelling {
        Spelling::Bracketed => value.as_str().and_then(bracket_pair).map(key_of),
        Spelling::RawPair => raw_pair(value).map(key_of),
        Spelling::Barcode => None,
    }
}

/// A grid pair in the export's canonical spelling, `"[7,1]"`.
pub fn key_of(pair: [i64; 2]) -> String {
    coordinate_key(pair)
}

/// One grid coordinate written back in the spelling a field uses: the bracketed
/// string, or the two-element array.
pub fn spell_coordinate(coordinate: &str, spelling: Spelling) -> Option<Value> {
    match spelling {
        Spelling::Bracketed => Some(Value::String(coordinate.to_string())),
        Spelling::RawPair => {
            let pair = bracket_pair(coordinate)?;
            Some(Value::Array(vec![Value::from(pair[0]), Value::from(pair[1])]))
        }
        Spelling::Barcode => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{coordinate_of, spell_coordinate, CellNamed, SourceIndex};
    use crate::records::Spelling;
    use serde_json::json;
    use uuid::Uuid;

    /// The three spellings reach one cell, which is the whole reason this module
    /// is a join table rather than four.
    #[test]
    fn all_three_spellings_of_one_cell_reach_the_same_identity() {
        let id = Uuid::parse_str("c0000000-0000-4000-8000-000000000001").expect("a UUID");
        let mut index = SourceIndex::default();
        index.add_cell("[6,1]", "042.913", id, [100.0, 200.0]);
        assert_eq!(index.cell(&json!("[6,1]"), Spelling::Bracketed), CellNamed::One(id));
        assert_eq!(index.cell(&json!([6, 1]), Spelling::RawPair), CellNamed::One(id));
        assert_eq!(index.cell(&json!("042.913"), Spelling::Barcode), CellNamed::One(id));
        assert_eq!(index.cell(&json!("998.997"), Spelling::Barcode), CellNamed::Nothing);
        assert_eq!(index.position(id), Some([100.0, 200.0]));
        assert_eq!(index.coordinate(id), Some("[6,1]"));
        // A respelling names nothing: the export would write the cell's own
        // spelling back.
        assert_eq!(index.cell(&json!("[6, 1]"), Spelling::Bracketed), CellNamed::Nothing);
        assert_eq!(index.cell(&json!([6.0, 1]), Spelling::RawPair), CellNamed::Nothing);
    }

    /// A record named by its key resolves only when the key is spelled in the
    /// JSON kind its field's file fixes, which the export writes it back in.
    #[test]
    fn a_key_names_a_record_only_spelled_as_the_export_writes_it_back() {
        use crate::records::{Join, KeyText};
        let id = Uuid::parse_str("c0000000-0000-4000-8000-000000000003").expect("a UUID");
        let mut index = SourceIndex::default();
        index.add_block("7", id);
        index.add_seat("31", "front-1", id);
        assert_eq!(index.joined(Join::Block, KeyText::Text, &json!("7"), ""), Some(id));
        assert_eq!(index.joined(Join::Block, KeyText::Text, &json!(7), ""), None);
        assert_eq!(index.joined(Join::Seat, KeyText::Text, &json!("front-1"), "31"), Some(id));
        assert_eq!(index.joined(Join::Seat, KeyText::Text, &json!("front-1"), "32"), None);
    }

    /// A barcode two cells carry names neither: which one a reference means is
    /// a guess. Two cells merely sharing one are no question until something
    /// names it.
    #[test]
    fn a_barcode_two_cells_carry_names_no_one_cell() {
        let ids = ["c0000000-0000-4000-8000-000000000001", "c0000000-0000-4000-8000-000000000002"]
            .map(|id| Uuid::parse_str(id).expect("a UUID"));
        let mut index = SourceIndex::default();
        index.add_cell("[6,1]", "042.913", ids[0], [0.0, 0.0]);
        index.add_cell("[7,1]", "042.913", ids[1], [900.0, 0.0]);
        assert_eq!(index.cell(&json!("042.913"), Spelling::Barcode), CellNamed::Shared("042.913".into()));
        assert_eq!(index.cell(&json!("[7,1]"), Spelling::Bracketed), CellNamed::One(ids[1]));
    }

    /// A coordinate written back comes back in the spelling its own field uses,
    /// not in whichever spelling it was read in.
    #[test]
    fn a_coordinate_is_written_back_in_its_fields_own_spelling() {
        assert_eq!(spell_coordinate("[6,1]", Spelling::Bracketed), Some(json!("[6,1]")));
        assert_eq!(spell_coordinate("[6,1]", Spelling::RawPair), Some(json!([6, 1])));
        assert_eq!(coordinate_of(&json!([6, 1]), Spelling::RawPair).as_deref(), Some("[6,1]"));
        assert_eq!(coordinate_of(&json!("[6,1]"), Spelling::Bracketed).as_deref(), Some("[6,1]"));
    }
}
