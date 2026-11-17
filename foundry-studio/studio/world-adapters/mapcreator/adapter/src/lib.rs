//! The Map Creator adapter: a Map Creator site export into a site's entities
//! and back out, **both directions, round-trippable**.
//!
//! Every GreyOrange site's map exists as a Map Creator site export, a folder of
//! JSON files. This module is the one place that knows what a Map Creator field
//! means, so that the library, the lens engine and the server can stay
//! ignorant of the format: a host hands it the files and the site, and it
//! answers with the changes that make the site say what the files say
//! (`ingest`); a host hands it the site, and it answers with the files
//! (`project`).
//!
//! Two mechanisms make the round trip exact, in this order of preference. A
//! fact the protocol already has a component for takes that component, exactly
//! as SPEC §10 defines it, and what Map Creator says beyond the core of a cell
//! or a presentation point is optional fields of the same component, which the
//! protocol's schema states. A fact the core cannot say takes a component of the
//! adapter's vocabulary — shared, undotted and ratified, save three under
//! `mapcreator.` at `experimental`: the export's table of contents, and the
//! frame and station facts whose merge is not obvious — and `vocabulary`
//! states every one of them with its meaning and its JSON Schema so a server can
//! register them without knowing them. How the files wrote each record rides
//! apart from all of these, in `import_formatting` (`written`).
//!
//! The read direction is `worldlens`: one lens per record kind, and a
//! serializer that turns the objects it projects into records. The write
//! direction is explicit constructors, because a lens is a query and a query's
//! inverse is not unique — many graphs project to the same objects. What holds
//! the two together is not a language feature but a pair of laws, stated once
//! for every adapter by `worldadapt-testkit`: ingest an export into a site,
//! apply the changes and project, and the same files come back; ingest the same
//! files again, and nothing changes.
//!
//! Identity is looked up, never computed: a record's entity is the one the site
//! already holds under the record's key in `external_ids`, and a record nothing
//! matches takes an identity the host issues (`identity`).

pub mod coverage;
pub mod dump;
pub mod export;
pub mod identity;
pub mod import;
pub mod ingest;
pub mod kinds;
pub mod layout;
pub mod ledger;
pub mod format;
pub mod ordering;
pub mod records;
pub mod redundancy;
pub mod references;
pub mod serialize;
pub mod site;
pub mod source_index;
pub mod vocabulary;
pub mod written;

use dump::Dump;
use identity::Existing;
use import::{import, Import};
use site::Held;
use worldadapt_sdk::{export_adapter, Adapter, Change, File, Ingest, Project, Site};

/// The adapter's own lens library, as text: every lens and every pattern in one
/// file, which is how the projection runs them.
pub fn lenses() -> &'static str {
    include_str!(concat!(env!("FOUNDRY_APPLICATION_ROOT"), "/data/seed/application/lenses/mapcreator.worldlens"))
}

/// The Map Creator adapter, as a host knows it.
struct MapCreator;

impl Adapter for MapCreator {
    const NAME: &'static str = "mapcreator";
    const VERSION: &'static str = "0.1.0";
    const FORMAT: &'static str = "Map Creator site export";
}

impl Project for MapCreator {
    fn project(site: &Site) -> Result<Vec<File>, String> {
        Ok(export::of_site(&Held::all_of(site))?.write())
    }
}

impl Ingest for MapCreator {
    fn ingest(files: &[File], site: &Site) -> Result<Vec<Change>, String> {
        let dump = Dump::read(files)?;
        let held = Held::all_of(site);
        site::owned(&held)?;
        let existing = Existing::of_entities(&held)?;
        let imported = import(&dump, &existing, identity::from_host())?;
        // The files are not needed past the import, and the comparison below
        // is the next large thing an ingest builds.
        drop(dump);
        refuse_what_the_ledger_cannot_account_for(&imported)?;
        let Import { entities, unclaimed, .. } = imported;
        ingest::changes(entities, &held, &existing, &unclaimed)
    }
}

export_adapter!(MapCreator: Project + Ingest);

/// What the crate this adapter was ported from counted in its ledger and let
/// pass, refused instead: a module has no channel for a ledger, and a count
/// nobody reads is data lost with nobody told.
///
/// A key path no disposition covers is a field family nobody has decided
/// about. A measured value no step states exactly, which the crate rounded and
/// counted here, the import refuses itself, naming it.
fn refuse_what_the_ledger_cannot_account_for(imported: &Import) -> Result<(), String> {
    let ledger = &imported.ledger;
    if !ledger.uncovered.is_empty() {
        let shown: Vec<&str> = ledger.uncovered.iter().take(ledger::INSTANCES).map(String::as_str).collect();
        let more = ledger.uncovered.len() - shown.len();
        return Err(format!(
            "the export carries {} key path(s) that no disposition covers, so where they would go is undecided: \
             {}{}",
            ledger.uncovered.len(),
            shown.join(", "),
            if more > 0 { format!(", and {more} more") } else { String::new() }
        ));
    }
    if !ledger.passes() {
        return Err(format!("the import does not account for the export: {}", ledger.headline()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use worldlens::Library;

    /// The library parses under the engine as it is, which is the first thing a
    /// server's publish gate asks of it.
    #[test]
    fn the_lens_library_parses() {
        Library::parse(super::lenses()).expect("the shipped lens library parses");
    }
}
