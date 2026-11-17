//! What the flow compiler's tests share: the two fixture documents loaded once through dessim's reader, small edits
//! to a flow scope, and the litmus study's setup.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use dessim::composition::flowgraph::{Code, Compiled, Container, OrderKind, Request, RunSetup, compile};
use dessim::model::ExitPath;
use dessim::worldmap::flows::{FlowEntity, FlowScope, read};
use dessim::worldmap::{WorldMap, load, load_from_bytes};
use serde_json::Value;
use site_model::ids::EntityId;

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flowgraph")
}

pub fn l_shape_snapshot() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/snapshots/worlds/flowgraph/l_shape_6_aisles.json")
}

pub fn json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|fault| panic!("{}: {fault}", path.display()))
}

/// The L-Shape made at 6 aisles (spec 06 section 6.1).
pub static L_SHAPE: LazyLock<WorldMap> =
    LazyLock::new(|| load(&l_shape_snapshot()).expect("the L-Shape fixture loads in dessim's reader"));

/// The store-replenishment litmus flow on DFW6's map (spec 06 section 9, statement 4).
pub static LITMUS: LazyLock<WorldMap> = LazyLock::new(|| {
    load(&fixtures().join("store_replenishment.json")).expect("the litmus fixture loads in dessim's reader")
});

/// The L-Shape document edited before it is loaded, for a test that needs the map or the fleet changed.
pub fn l_shape_with(edit: impl FnOnce(&mut Value)) -> WorldMap {
    let mut document = json(&l_shape_snapshot());
    edit(&mut document);
    load_from_bytes(&serde_json::to_vec(&document).unwrap()).expect("the edited L-Shape loads")
}

pub fn flows_of(world: &WorldMap) -> FlowScope {
    read(&world.world)
}

pub fn compile_l_shape(request: &Request) -> Compiled {
    compile(&L_SHAPE, &flows_of(&L_SHAPE), request)
}

/// Every order kind the litmus study's workload holds: `walmart-data/outbound.csv` ships each of bag, box and SIOC
/// (a tray under the study's `unknown_container: as_tray`) to each of DS, SC and 3P (buffer, sorter, dock).
pub fn study_order_kinds() -> Vec<OrderKind> {
    [Container::Tote, Container::Tray, Container::Carton]
        .into_iter()
        .flat_map(|container| {
            [ExitPath::Buffer, ExitPath::Sorter, ExitPath::Dock].map(|exit_path| OrderKind {
                container,
                exit_path,
                routing: None,
            })
        })
        .collect()
}

/// The litmus study's run mode: 128 order trays and half the stock at the start (`walmart_dfw6_throughput_spec.json`).
pub fn study_run() -> Request {
    Request::run(RunSetup { stock_at_start: 0.5, order_trays: 128 }, study_order_kinds())
}

/// The flow entity whose `name` or derived label is `label`.
pub fn id_of(compiled: &Compiled, flows: &FlowScope, label: &str) -> EntityId {
    let found: Vec<&FlowEntity> = flows.entities.iter().filter(|e| compiled.labels.of(&e.id) == label).collect();
    assert_eq!(found.len(), 1, "one flow entity is labelled {label:?}");
    found[0].id.clone()
}

/// Change one flow entity's component in place.
pub fn edit(flows: &mut FlowScope, id: &EntityId, component: &str, change: impl FnOnce(&mut Value)) {
    let entity = flows.get_mut(id).unwrap_or_else(|| panic!("{id} is a flow entity"));
    change(entity.components.get_mut(component).unwrap_or_else(|| panic!("{id} carries {component}")));
}

/// Add a flow entity, keeping the scope in identity order.
pub fn add(flows: &mut FlowScope, id: &str, name: Option<&str>, component: &str, value: Value) -> EntityId {
    let id: EntityId = id.parse().unwrap();
    let entity = FlowEntity {
        id: id.clone(),
        name: name.map(str::to_owned),
        components: [(component.to_owned(), value)].into(),
    };
    flows.entities.push(entity);
    flows.entities.sort_by(|a, b| a.id.cmp(&b.id));
    id
}

pub fn remove(flows: &mut FlowScope, id: &EntityId) {
    flows.entities.retain(|entity| &entity.id != id);
}

/// The entities the problems of `code` name, in order.
pub fn named_by(compiled: &Compiled, code: Code) -> Vec<EntityId> {
    compiled
        .problems
        .iter()
        .filter(|p| p.code == code)
        .map(|p| p.reference.clone().expect("the problem names an entity"))
        .collect()
}
