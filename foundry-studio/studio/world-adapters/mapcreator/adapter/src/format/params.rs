//! Within `format`, `params` is the **invented-value register**: every
//! value the world protocol needs that no Map Creator export states.
//!
//! It exists so that "the export never said this" can never be answered by a
//! literal buried in import code. A Map Creator dump is a survey, so it invents
//! far less than a converted design fixture does — most of what the protocol
//! wants is measured and present. What it does need falls in two families, and
//! both are recorded here with a written reason: constants the export omits (the
//! floor's elevation), and **values recovered by regression over the site's own
//! cells** because the export states a projection where it should state a
//! transform (the grid frame's pitch, the vendor frame's offsets). The second
//! family is the more important one: SPEC §4 calls `vda5050_reference` the
//! anti-pattern by name — a single sample point offered in place of a transform —
//! and recovering the transform is what this importer has to do before it can
//! declare one.
//!
//! Every value an import reaches for lands in that site's ledger, and the
//! provenance sentence lands beside the number in the emitted document's
//! `$comment`.

use std::cell::RefCell;
use std::collections::BTreeMap;

/// One invented value, as it appears in a ledger: what it is, what it was set
/// to, and why that value rather than another.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UsedParam {
    pub name: &'static str,
    pub value: String,
    pub provenance: &'static str,
}

/// The values an import may need and the export cannot supply.
#[derive(Debug, Clone)]
pub struct Params {
    /// Where the floor frame sits inside the 3-D site frame. Every dump seen
    /// carries `floor_id: 1` and an empty `elevator.json`, so no export states
    /// the elevation of anything.
    pub floor_elevation_mm: f64,
    used: RefCell<BTreeMap<&'static str, UsedParam>>,
}

impl Default for Params {
    fn default() -> Params {
        Params { floor_elevation_mm: 0.0, used: RefCell::new(BTreeMap::new()) }
    }
}

impl Params {
    /// Record that an import reached for one of these values, and hand back the
    /// sentence that goes in the emitted `$comment`. Nothing invented reaches a
    /// document without passing through here.
    pub fn note(&self, name: &'static str, value: impl ToString) -> &'static str {
        let provenance = provenance_of(name);
        self.used.borrow_mut().insert(name, UsedParam { name, value: value.to_string(), provenance });
        provenance
    }

    /// Everything this import invented, in name order, for the ledger.
    pub fn used(&self) -> Vec<UsedParam> {
        self.used.borrow().values().cloned().collect()
    }
}

/// The written reason for one invented value, for a caller that wants the
/// sentence without recording a use — a document `$comment` beside a value the
/// same import already recorded once, listing every place it applied.
pub fn provenance(name: &str) -> &'static str {
    provenance_of(name)
}

/// The written reason for each invented value. Kept beside the struct rather than
/// inside it so the same sentence lands in the ledger and in the document.
fn provenance_of(name: &str) -> &'static str {
    match name {
        "floor_elevation_mm" => {
            "Every dump carries floor_id 1 and an empty elevator.json, so no export states how \
             high a floor sits; the single floor is placed at the site frame's z origin."
        }
        "grid_frame_transform" => {
            "The export states a grid coordinate and a millimetre coordinate per cell and no \
             relation between them — the pitch varies cell to cell (site-b carries 582 distinct \
             size_info tuples), so no exact transform exists. This one is a least-squares fit over \
             the site's own on-grid cells, declared for projection only, exactly the nominal and \
             lossy grid frame SPEC §4 provides for; each cell's recorded grid coordinate rides in \
             external_ids, so the exact legacy value is never the fit's to lose."
        }
        "vendor_frame_transform" => {
            "vda_world_coordinate is stored per cell and the constants relating it to the world \
             frame appear nowhere in the file — vda5050_reference offers one sample point instead \
             of a transform (SPEC §4 names that the anti-pattern). Recovered by taking the modal \
             (Kx, Ky) over every cell of the site, as vda_x = world_x + Kx, vda_y = Ky - world_y; \
             the residual cells are counted in this ledger's impedance list."
        }
        "robot_class_role" => {
            "The export names bot types as opaque strings (HTM_QT_X1, VTM_HAI_Z9_E1) and \
             never states a role; the leading HTM/VTM is read as the role, which is the same split \
             butler compiles into ?HTM_BOTS / ?VTM_BOTS."
        }
        "robot_class_manufacturer" => {
            "No export carries a vendor field. The manufacturer is read out of the bot-type string \
             (QT = Quicktron, HAI = Hai Robotics), which is the sniffing the new format is meant \
             to end — so it is recorded as invented rather than as read."
        }
        "station_kind" => {
            "SPEC §10 gives a station one kind from pick/put/induct/extract; the export gives a \
             product type plus a list of allowed_modes, and the two do not line up one to one. \
             The rule: a type naming induction or extraction decides it, otherwise a single \
             pick-or-put mode decides it, otherwise the station's kind is not derivable and it is \
             recorded as an impedance rather than guessed at."
        }
        other => unreachable!("no provenance written for parameter `{other}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::Params;

    #[test]
    fn a_value_is_only_recorded_once_it_is_reached_for() {
        let params = Params::default();
        assert!(params.used().is_empty());
        params.note("floor_elevation_mm", 0.0);
        params.note("floor_elevation_mm", 0.0);
        assert_eq!(params.used().len(), 1);
        assert_eq!(params.used()[0].value, "0");
    }

    #[test]
    fn every_registered_name_has_a_written_reason() {
        let params = Params::default();
        for name in [
            "floor_elevation_mm",
            "grid_frame_transform",
            "vendor_frame_transform",
            "robot_class_role",
            "robot_class_manufacturer",
            "station_kind",
        ] {
            assert!(!params.note(name, "x").is_empty(), "{name}");
        }
    }
}
