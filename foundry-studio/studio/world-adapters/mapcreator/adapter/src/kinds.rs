//! Within the Map Creator adapter, `kinds` is **the export's file list, named once**: one
//! variant per JSON file a Map Creator dump may carry, and the record kind each
//! file's contents are.
//!
//! It exists so that nothing else in the adapter spells a file name as a string.
//! A dump has no manifest, so the set of files IS the format's table of
//! contents: the importer walks it to find what a site shipped, the exporter
//! walks it to decide what to write, and the round-trip laws compare the files
//! by these names. A typo in a file name would otherwise be a file silently
//! skipped.
//!
//! Two spellings of one file are two variants on purpose: `sectors.json` and
//! `sector.json` are the same record kind under two names, and which name a
//! site used has to come back out.

use std::fmt;

/// One file of a Map Creator export.
///
/// The order is the order butler's own upload sequence posts them — zones
/// before the map, the map before everything that references cells — which is
/// also the order a rebuilt dump is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileKind {
    Zone,
    Map,
    Block,
    Pps,
    ToteStorage,
    ToteType,
    Conveyor,
    ConveyorMapping,
    Charger,
    Elevator,
    VtmZone,
    DockPoint,
    OdsExcluded,
    /// The plural spelling, on every site but the oldest dump.
    Sectors,
    /// The singular spelling, which only `site-e` uses.
    Sector,
    SectorMxUPreferences,
    FireEmergency,
    QueueData,
    OpcConnection,
    ZoneController,
    PpsBin,
    PpsBinType,
    PpsSeat,
    Pptl,
}

impl FileKind {
    /// Every file kind, in upload order.
    pub const ALL: [FileKind; 24] = [
        FileKind::Zone,
        FileKind::Map,
        FileKind::Block,
        FileKind::Pps,
        FileKind::ToteStorage,
        FileKind::ToteType,
        FileKind::Conveyor,
        FileKind::ConveyorMapping,
        FileKind::Charger,
        FileKind::Elevator,
        FileKind::VtmZone,
        FileKind::DockPoint,
        FileKind::OdsExcluded,
        FileKind::Sectors,
        FileKind::Sector,
        FileKind::SectorMxUPreferences,
        FileKind::FireEmergency,
        FileKind::QueueData,
        FileKind::OpcConnection,
        FileKind::ZoneController,
        FileKind::PpsBin,
        FileKind::PpsBinType,
        FileKind::PpsSeat,
        FileKind::Pptl,
    ];

    /// The file's name on disk, which is also the key it takes in a dump read
    /// from memory.
    pub fn name(self) -> &'static str {
        match self {
            FileKind::Zone => "zone.json",
            FileKind::Map => "map.json",
            FileKind::Block => "block.json",
            FileKind::Pps => "pps.json",
            FileKind::ToteStorage => "tote_storage.json",
            FileKind::ToteType => "tote_type.json",
            FileKind::Conveyor => "conveyor.json",
            FileKind::ConveyorMapping => "conveyor_mapping.json",
            FileKind::Charger => "charger.json",
            FileKind::Elevator => "elevator.json",
            FileKind::VtmZone => "vtm_zone.json",
            FileKind::DockPoint => "dock_point.json",
            FileKind::OdsExcluded => "ods_excluded.json",
            FileKind::Sectors => "sectors.json",
            FileKind::Sector => "sector.json",
            FileKind::SectorMxUPreferences => "sectorMxUPreferences.json",
            FileKind::FireEmergency => "fire_emergency.json",
            FileKind::QueueData => "queue_data.json",
            FileKind::OpcConnection => "opc_connection.json",
            FileKind::ZoneController => "zone_controller.json",
            FileKind::PpsBin => "pps_bin.json",
            FileKind::PpsBinType => "pps_bintype.json",
            FileKind::PpsSeat => "pps_seat.json",
            FileKind::Pptl => "pptl.json",
        }
    }

    /// The kind a file name names, or `None` for a name no Map Creator export
    /// has ever carried.
    pub fn of(name: &str) -> Option<FileKind> {
        FileKind::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

impl fmt::Display for FileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::FileKind;

    /// The importer's own file list is the one this enum stands for, so the two
    /// have to name the same files: a name in one and not the other is a file
    /// one half of the adapter would silently skip.
    #[test]
    fn every_file_the_dump_reader_knows_has_a_kind() {
        for name in crate::format::source::EXPORT_FILES {
            assert!(FileKind::of(name).is_some(), "no FileKind for {name}");
        }
        assert_eq!(
            FileKind::ALL.len(),
            crate::format::source::EXPORT_FILES.len(),
            "the two file lists have drifted apart"
        );
    }

    #[test]
    fn no_two_kinds_share_a_name() {
        let mut names: Vec<&str> = FileKind::ALL.iter().map(|k| k.name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }
}
