//! Within `write`, `scenario` is WHAT A RECORDING STATES BEFORE ITS FIRST
//! SLICE: the scenario block, which names the run, the documents it was built
//! from, the world's ids, the robots and every metric and table the run
//! declares from the start, and the producer's own document, which carries
//! what the run model needs back that the format has no field for.

use std::collections::HashMap;

use desgraph::RobotType;
use desunits::{RobotId, SimDuration};
use serde::{Deserialize, Serialize};
use worldview::frames::scenario::{Document, RobotEntry, Scenario};
use worldview::frames::section_declarations;
use worldview::frames::tables::{TableDescriptor, TableId};

use super::producer::Producer;
use super::{DEFAULT_KEYFRAME_CADENCE_SLICES, RecordingError};
use crate::config::SimConfig;
use crate::metrics::profile::{
    IdTables, ProfileMetadata, ResolvedWorkload, RunDocuments, ScenarioSource, VtmFleet, robot_identities,
};
use crate::metrics::streams::StreamDescriptor;
use crate::metrics::worldview::journals;
use crate::metrics::worldview::summary::document;
use crate::metrics::worldview::wire::table_columns;
use crate::sim_spec::SLICE_WIDTH_MS;

/// What the run model needs back that no field of the format carries.
///
/// The format speaks a run's ids, its metrics and its rows; it has no opinion
/// about how the simulator described its own scenario or what the window it
/// drew was. Those ride here, in the producer's own MessagePack, and only
/// [`worldview::read`](crate::metrics::worldview::read) ever opens it.
#[derive(Clone, Serialize, Deserialize)]
pub struct ProducerDocument {
    pub name: String,
    pub branch: String,
    pub commit_short: String,
    pub grid_width: u32,
    pub grid_depth: u32,
    pub scenario_source: ScenarioSource,
    /// The SKUs and orders the scenario resolved to, and the DA50 fleet the
    /// world gave the run: facts about the run that no frame carries and that
    /// the workbook lists. Defaulted, so a recording written before they
    /// existed still opens.
    #[serde(default)]
    pub workload: ResolvedWorkload,
    #[serde(default)]
    pub vtm_fleet: VtmFleet,
    /// Each table's full stream descriptor — the labels, the per-column
    /// descriptions and the context the format's own four fields leave out —
    /// so a journal comes back describing itself as it went in.
    pub tables: Vec<StreamDescriptor>,
}

/// What a recording states before the run's first instant that the observer
/// never learns: what the run is called and on which code it ran, the
/// documents it was built from, the ids the world document names, its
/// configuration and workload, and which run and try at it a server with a
/// registry behind it says this is (desvis has neither and states neither).
pub struct RunScenario {
    pub name: String,
    pub metadata: ProfileMetadata,
    pub documents: RunDocuments,
    /// The world's ids; the robots come from the observer's roster.
    pub ids: IdTables,
    pub sim_config: SimConfig,
    pub scenario: ScenarioSource,
    pub workload: ResolvedWorkload,
    pub vtm_fleet: VtmFleet,
    pub run: String,
    pub attempt: u32,
}

pub(super) fn scenario_block(
    scenario: &RunScenario,
    producer: &Producer,
    tables: &[TableDescriptor],
    streams: &[StreamDescriptor],
    roster: &HashMap<RobotId, RobotType>,
) -> Result<Scenario, RecordingError> {
    let RunScenario { name, metadata, documents, ids, sim_config, run, attempt, .. } = scenario;
    let document_of_producer = ProducerDocument {
        name: name.clone(),
        branch: metadata.branch.clone(),
        commit_short: metadata.commit_short.clone(),
        grid_width: metadata.grid_width,
        grid_depth: metadata.grid_depth,
        scenario_source: scenario.scenario.clone(),
        // A loaded scenario IS its SKU and order lists, so stating the
        // workload beside it would carry both twice.
        workload: match scenario.scenario {
            ScenarioSource::Loaded { .. } => ResolvedWorkload::default(),
            ScenarioSource::Parametric(_) => scenario.workload.clone(),
        },
        vtm_fleet: scenario.vtm_fleet,
        tables: streams.to_vec(),
    };
    Ok(Scenario {
        // A desvis run has no registry behind it and states neither; a server
        // with one states both, so a recording says which run it is even when
        // it has been copied away from the key it was stored under.
        run: run.clone(),
        attempt: *attempt,
        run_name: name.clone(),
        commit: metadata.commit.clone(),
        dirty: metadata.dirty,
        timestamp: metadata.timestamp.clone(),
        hostname: metadata.hostname.clone(),
        // The width the observer actually slices at, which is a constant of
        // its own and not `SimConfig::slice_width_ms`. Stating the config's
        // number would have a reader place every slice by a clock the run
        // never ran on.
        slice_width: SimDuration::ms(SLICE_WIDTH_MS),
        keyframe_cadence_slices: DEFAULT_KEYFRAME_CADENCE_SLICES,
        world: Document(documents.world.clone()),
        scenario: Document(documents.spec.clone()),
        data: Some(documents.data.iter().map(|(name, bytes)| (name.clone(), Document(bytes.clone()))).collect()),
        sim_config: document(sim_config, "simulation config")?,
        producer: Some(document(&document_of_producer, "producer document")?),
        frame: ids.frame.clone(),
        cells: ids.cells.clone(),
        stations: ids.stations.clone(),
        robots: robot_identities(roster)
            .into_iter()
            .map(|robot| RobotEntry {
                robot: robot.robot,
                name: robot.name,
                class: robot.class.into(),
                class_entity: robot.class_entity,
            })
            .collect(),
        metrics: producer.declared().to_vec(),
        tables: tables.to_vec(),
        // The observer states every robot's state in every slice.
        sections: section_declarations(true),
        ..Scenario::default()
    })
}

/// Every journal the run keeps, as a table and as the descriptor the reader
/// rebuilds the stream from. Their order fixes each table's id for the run.
///
/// A journal with no rows anywhere still gets a descriptor, so a reader can
/// tell an empty journal from one nobody wrote.
pub(super) fn journal_tables() -> Result<(Vec<TableDescriptor>, Vec<StreamDescriptor>), RecordingError> {
    let declared = journals::declarations().map_err(RecordingError::Journals)?;
    let mut tables = Vec::with_capacity(declared.len());
    let mut streams = Vec::with_capacity(declared.len());
    for (index, stream) in declared.iter().enumerate() {
        let columns = table_columns(&stream.descriptor.columns);
        tables.push(TableDescriptor {
            id: TableId(index as u16 + 1),
            name: stream.descriptor.id.clone(),
            instant_column: journals::instant_column(&stream.descriptor.id),
            columns,
        });
        streams.push(stream.descriptor.clone());
    }
    Ok((tables, streams))
}
