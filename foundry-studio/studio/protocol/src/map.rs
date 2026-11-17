//! Within `studio-protocol`, `map` is spec 03 sections 3.5 and 3.6's payloads: what the Map panel reads of a site at
//! a cut (`MapSummary`, `GET /api/map`), the marks a reference covers on a panel (`Mark`), `POST /api/resolve`'s
//! request and answer, and `GET /api/find`'s answer. Positions are whole millimetres of the site's primary frame.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::grammar::{EntityId, Key, KinPins, Location, Ref, ScopePath, SitePath};
use crate::made_from::MadeFrom;
use crate::panel::{Panel, ScopeSeq};
use crate::problem::Problem;
use crate::template_evaluation::FloorRef;

/// Whole millimetres along one axis of a site's primary frame: a number in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(type = "number", export_to = "map.ts")]
pub struct Mm(i64);

impl Mm {
    pub const fn new(mm: i64) -> Self {
        Self(mm)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

/// What one step of a frame is worth in millimetres, as world-proto's primary frame declares it in `unit_mm` (0.1 by a
/// site's default): a frame's scale, fractional by design and never a position, so a quantity of its own beside the
/// whole millimetres positions are given in. Always finite and above zero; a number in JSON.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, TS)]
#[serde(try_from = "f64", into = "f64")]
#[ts(type = "number", export_to = "map.ts")]
pub struct UnitMm(f64);

impl UnitMm {
    /// `mm` millimetres a step, or none for what no step can be worth: not finite, or not above zero.
    pub fn new(mm: f64) -> Option<Self> {
        (mm.is_finite() && mm > 0.0).then_some(Self(mm))
    }

    pub const fn mm(self) -> f64 {
        self.0
    }
}

// Equality is total: no value is NaN, since `new` and deserialization both refuse it.
impl Eq for UnitMm {}

impl TryFrom<f64> for UnitMm {
    type Error = String;

    fn try_from(mm: f64) -> Result<Self, String> {
        Self::new(mm).ok_or_else(|| format!("{mm} mm is not what a frame's step can be worth"))
    }
}

impl From<UnitMm> for f64 {
    fn from(unit: UnitMm) -> Self {
        unit.0
    }
}

/// How many entities: a number in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(type = "number", export_to = "map.ts")]
pub struct EntityCount(u64);

impl EntityCount {
    pub const fn new(count: u64) -> Self {
        Self(count)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The site a summary reads: its root scope, and its name from the root's prologue, else the root's path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "map.ts")]
pub struct MapSite {
    pub scope: SitePath,
    pub label: String,
}

/// The frame positions are given in: the prologue's primary frame, and the millimetres in one of its units.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "map.ts")]
pub struct MapFrame {
    pub id: EntityId,
    pub unit_mm: UnitMm,
}

/// The frame whose axes the stage draws in a corner and the text names: the frame most of the map is stated in (a
/// template-made site's shell frame, which its wall knobs name), else the primary; and where its x and y point on the
/// drawing, which shows the primary frame north-up, x to the right and y up the drawing (`[1, 0]` and `[0, 1]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "map.ts")]
pub struct FrameAxes {
    pub frame: EntityId,
    pub name: String,
    pub x: [i32; 2],
    pub y: [i32; 2],
}

/// One row of "What the site holds": the entities carrying exactly one set of components, discovered, never listed
/// ahead; labelled by the component names joined with " + ".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "map.ts")]
pub struct CountRow {
    pub label: String,
    pub components: Vec<String>,
    pub count: EntityCount,
}

/// `GET /api/map/{*location}?kin=`'s answer (spec 03 section 3.6): the cut read, every scope it exported; the site;
/// the floor's key, or, when the document loaded but has no floor, `floor_problem` saying why (`floor-compile-failed`);
/// `empty` when the map scope holds nothing at this cut; the frames; the counts; the made-from record of the cut; and
/// whether every read was exact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "map.ts")]
pub struct MapSummary {
    pub location: Location,
    pub kin: Option<KinPins>,
    pub resolved: Vec<ScopeSeq>,
    pub site: MapSite,
    pub floor: Option<FloorRef>,
    pub floor_problem: Option<Problem>,
    pub empty: bool,
    pub frame: MapFrame,
    pub axes: FrameAxes,
    pub counts: Vec<CountRow>,
    pub made_from: Option<MadeFrom>,
    pub exact: bool,
    pub exact_reason: Option<String>,
}

/// One thing a reference covers on the panel in view (spec 03 section 3.5): its key, its label for people only, its
/// components, its centre when it stands somewhere, and whether it stands on floor cells, which only those outline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "map.ts")]
pub struct Mark {
    pub key: Key,
    pub label: String,
    pub components: Vec<String>,
    pub centre_mm: Option<[Mm; 2]>,
    pub on_floor: bool,
}

/// `POST /api/resolve`'s body. The strings stay as written, so a malformed one is named (`ref-malformed`); `location`
/// may be left out for a list panel, and `draft` overlays a flow draft (spec 06's rows).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "map.ts", optional_fields)]
pub struct ResolveRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
    pub panel: Panel,
    pub refs: Vec<String>,
}

/// One reference resolved: what it names, the scope that owns it (none for a key of Postgres's), its label and
/// components, and its marks in mark order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "map.ts")]
pub struct RefMarks {
    #[serde(rename = "ref")]
    pub ref_: Ref,
    pub key: Key,
    pub scope: Option<ScopePath>,
    pub label: String,
    pub components: Vec<String>,
    pub marks: Vec<Mark>,
}

/// `POST /api/resolve`'s answer: the cut read, and each reference in the order asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "map.ts")]
pub struct Resolution {
    pub resolved: Vec<ScopeSeq>,
    pub refs: Vec<RefMarks>,
}

/// `GET /api/find/{*location}?q=`'s answer: the first matches in mark order, and whether more matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "map.ts")]
pub struct FindResults {
    pub items: Vec<Mark>,
    pub more: bool,
}
