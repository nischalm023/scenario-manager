//! Within desvis, `entity_panel` is the ENTITY EXPLORER: the docked roll of
//! every entity in the loaded world document — each declared entity with the
//! entities derived from it indented beneath it, each component a lozenge that
//! opens the component's own body — together with the outline it draws on the
//! map for the row under the pointer.
//!
//! It exists so a converted site can be read back against its document. The map
//! shows floor, steel, relays and stations; this says which entity each of them
//! IS, what the document makes it out of, and which entity derived it. Hovering
//! points at the map; clicking a lozenge opens that component's body from the
//! retained document; nothing here edits or competes with the click selection
//! the inspector owns.

use std::ops::Range;
use std::sync::Arc;

use eframe::egui;
use rustc_hash::{FxHashMap, FxHashSet};

use dessim::worldmap::World;
use dessim::worldmap::census::{EntityPlace, EntityRow};

use crate::app::DesvisApp;
use worldview_render::CameraState;

use crate::screen;
use crate::theme;
use crate::types::CellInfo;

/// The panel's width when it first opens, and the band the drag handle allows.
/// Wide enough that a derivation key survives beside a name and a set of
/// component lozenges.
const DEFAULT_WIDTH: f32 = 520.0;
const MIN_WIDTH: f32 = 340.0;
const MAX_WIDTH: f32 = 900.0;

/// One row's height, fixed because the body is virtualized: `show_rows` turns
/// scroll offset into a row index by dividing, so a row that draws taller than
/// this scrolls out of step with its own scrollbar. Tall enough for a lozenge.
const ROW_HEIGHT: f32 = 22.0;

/// How far a derived row's id starts right of its declared parent's. The
/// indent IS the provenance column: an id at the margin is declared, an id
/// stepped in is derived from the row it sits under.
const INDENT: f32 = 16.0;

/// How much of a UUID a row shows. The full identity is in the hover.
const ID_PREFIX: usize = 8;

/// The roll of the loaded document, and what the pointer is doing to it.
pub(crate) struct EntityPanel {
    /// Whether the panel is docked open. The sidebar's Entities button flips
    /// it.
    pub(crate) open: bool,
    filter: String,
    /// Every entity, declared before derived and then by identity — the
    /// census's own order, which is what makes each declared entity's derived
    /// rows a contiguous run (`Group.children`).
    rows: Vec<Row>,
    /// The document the rows were read from, for a lozenge's component body.
    /// `None` only when a session has rows from a source that kept no
    /// document.
    world: Option<Arc<World>>,
    /// One per declared row, in row order: the tree's top level.
    groups: Vec<Group>,
    /// Derived rows whose root is not a declared row of this document. Shown
    /// flat at the end; on a well-formed document there are none.
    orphans: Vec<u32>,
    /// Every distinct component name with how many entities carry it, sorted
    /// by name. Clicking one filters the roll to entities carrying all active
    /// names.
    chips: Vec<Chip>,
    declared: usize,
    derived: usize,
    /// Group indices whose derived rows are unfolded.
    expanded: FxHashSet<u32>,
    /// Group indices a click folded shut while a filter was auto-unfolding
    /// them. Cleared when the filter changes: a fresh search unfolds fresh.
    collapsed: FxHashSet<u32>,
    /// What the body draws, in order: rebuilt when the filter text, the active
    /// chips, or the folding changed — never per frame, a site's roll runs to
    /// six figures.
    visible: Vec<VisRow>,
    visible_dirty: bool,
    shown_declared: usize,
    shown_derived: usize,
    /// The row the pointer is over. Written while the panel draws and read by
    /// the map overlay later in the SAME frame, so the outline never trails
    /// the pointer.
    hovered: Option<u32>,
    /// The row a click pinned. A pinned row stays outlined on the map after
    /// the pointer leaves it; clicking it again lets it go.
    pinned: Option<u32>,
    /// The component body a lozenge opened, if one is open.
    detail: Option<Detail>,
    /// `CellId.0` → the row that IS that cell entity: the map's hover and
    /// click resolve through this.
    cell_row: FxHashMap<u32, u32>,
    /// Every rect- or prism-placed row as `(area, rect_mm, top_mm, row)`,
    /// sorted smallest-area first. Flat places carry `top_mm == 0` and answer
    /// at the ray's ground crossing; standing steel answers wherever the ray
    /// meets its box.
    prisms: Vec<(f32, [f64; 4], f64, u32)>,
    /// Every shelved row's pick box — slots on their shelves, boards at
    /// their heights — as `(rect_mm, z_mm, row)`. A slot's box is a small
    /// square at its centre; a board's is stretched outward from its bay
    /// toward the aisle, covering the tab the renderer actually draws (the
    /// converted geometry's stand-to-face span). Ray-tested only inside bays
    /// the ray already hit, so a hundred thousand slots cost nothing until a
    /// rack is actually under the cursor.
    shelf_points: Vec<([f64; 4], f64, u32)>,
    /// The rows the last ⌥-click's ray passed through, nearest first. While
    /// set, the list shows exactly these (the text and chip filters narrow
    /// within them); cleared from the banner above the table.
    ray_filter: Option<Vec<u32>>,
    /// A derived root's UUID text → its group, for unfolding the parent of a
    /// row the map opened.
    group_of_root: FxHashMap<String, u32>,
    /// One-shot: scroll the tree to this row on the next draw.
    scroll_to: Option<u32>,
}

/// One entity as the list shows it.
struct Row {
    id_text: String,
    /// The id as the column shows it: a declared UUID cut to its leading
    /// characters; a derived identity cut to `/key`, because its root is the
    /// row it is indented under.
    short_id: String,
    name: Option<String>,
    components: Arc<[Arc<str>]>,
    derived: bool,
    place: EntityPlace,
}

/// One declared row and the contiguous run of derived rows rooted in it.
struct Group {
    row: u32,
    children: Range<u32>,
}

/// One component name as the filter bar offers it.
struct Chip {
    name: Arc<str>,
    count: usize,
    active: bool,
}

/// One row of the flattened, filtered tree.
struct VisRow {
    row: u32,
    kind: VisKind,
}

enum VisKind {
    /// A declared row: its group, how much of its derivation the current
    /// filter admits, and whether that derivation is unfolded beneath it.
    /// `unfolded` is the flattened tree's own truth — the chevron draws it
    /// and a click toggles against it, so glyph and behaviour cannot split.
    Parent { group: u32, shown_children: usize, total_children: usize, unfolded: bool },
    /// A derived row, indented under the parent above it.
    Child,
    /// An orphan or a rootless roll: drawn at the margin with no chevron.
    Flat,
}

/// The component body a lozenge opened.
struct Detail {
    row: u32,
    component: Arc<str>,
    /// The body pretty-printed once at click time, not per frame.
    body: String,
}

/// The column widths, worked out once from the width the panel was handed.
/// The header and every body row take the same `Columns`, which is what holds
/// the two in line; a row that sized itself from its own contents is how a
/// column comes out ragged.
#[derive(Clone, Copy)]
struct Columns {
    id: f32,
    name: f32,
    components: f32,
}

impl Columns {
    /// `spacing` is the gap the horizontal layout puts between two cells, and
    /// there are two of them across three columns.
    fn of(width: f32, spacing: f32) -> Columns {
        let id = (width * 0.26).clamp(110.0, 170.0);
        let components = (width * 0.44).clamp(150.0, 460.0);
        let name = (width - id - components - spacing * 2.0).max(0.0);
        Columns { id, name, components }
    }
}

impl Default for EntityPanel {
    /// The panel a session with no world document gets: the profile-only
    /// paths, and a live session whose map came from somewhere else.
    fn default() -> EntityPanel {
        EntityPanel {
            open: false,
            filter: String::new(),
            rows: Vec::new(),
            world: None,
            groups: Vec::new(),
            orphans: Vec::new(),
            chips: Vec::new(),
            declared: 0,
            derived: 0,
            expanded: FxHashSet::default(),
            collapsed: FxHashSet::default(),
            visible: Vec::new(),
            visible_dirty: true,
            shown_declared: 0,
            shown_derived: 0,
            hovered: None,
            pinned: None,
            detail: None,
            cell_row: FxHashMap::default(),
            prisms: Vec::new(),
            shelf_points: Vec::new(),
            ray_filter: None,
            group_of_root: FxHashMap::default(),
            scroll_to: None,
        }
    }
}

impl EntityPanel {
    /// Take the loader's roll of the document, and the document itself for
    /// the lozenges' component bodies.
    pub(crate) fn from_census(census: Vec<EntityRow>, world: Option<Arc<World>>) -> EntityPanel {
        let mut declared = 0;
        let mut chip_counts: Vec<(Arc<str>, usize)> = Vec::new();
        let rows: Vec<Row> = census
            .into_iter()
            .map(|row| {
                let derived = row.id.is_derived();
                if !derived {
                    declared += 1;
                }
                for name in row.components.iter() {
                    match chip_counts.binary_search_by(|(n, _)| n.as_ref().cmp(name.as_ref())) {
                        Ok(at) => chip_counts[at].1 += 1,
                        Err(at) => chip_counts.insert(at, (Arc::clone(name), 1)),
                    }
                }
                let id_text = row.id.to_string();
                Row {
                    short_id: short_id(&id_text, derived),
                    id_text,
                    name: row.name,
                    components: row.components,
                    derived,
                    place: row.place,
                }
            })
            .collect();

        // Declared rows sort ahead of derived ones and derived rows sort by
        // (root, key), so each declared row's derivation is one contiguous
        // run. Walk the derived tail once, cutting it at root changes.
        let mut groups: Vec<Group> =
            rows[..declared].iter().enumerate().map(|(index, _)| Group { row: index as u32, children: 0..0 }).collect();
        let root_of_declared: FxHashMap<String, u32> =
            rows[..declared].iter().enumerate().map(|(index, row)| (row.id_text.clone(), index as u32)).collect();
        let mut orphans: Vec<u32> = Vec::new();
        let mut cursor = declared as u32;
        while (cursor as usize) < rows.len() {
            let root = derived_root(&rows[cursor as usize].id_text);
            let start = cursor;
            while (cursor as usize) < rows.len() && derived_root(&rows[cursor as usize].id_text) == root {
                cursor += 1;
            }
            match root_of_declared.get(root) {
                Some(&group) => groups[group as usize].children = start..cursor,
                None => orphans.extend(start..cursor),
            }
        }

        // The map's two ways in: a cell id straight to its cell entity, and
        // the pick ray through the rect and prism places, smallest area first
        // so the most specific entity standing somewhere answers first among
        // equals.
        let mut cell_row: FxHashMap<u32, u32> = FxHashMap::default();
        let mut prisms: Vec<(f32, [f64; 4], f64, u32)> = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            match &row.place {
                EntityPlace::Cells(cells)
                    if cells.len() == 1 && row.components.iter().any(|name| name.as_ref() == "cell") =>
                {
                    cell_row.insert(cells[0].0, index as u32);
                }
                EntityPlace::Rect(rect) => {
                    let [_, _, w, h] = *rect;
                    prisms.push(((w * h) as f32, *rect, 0.0, index as u32));
                }
                EntityPlace::Prism { rect, top_mm } => {
                    let [_, _, w, h] = *rect;
                    prisms.push(((w * h) as f32, *rect, *top_mm, index as u32));
                }
                _ => {}
            }
        }
        prisms.sort_by(|a, b| a.0.total_cmp(&b.0));

        // Shelved pick boxes, after the prisms exist: a board's box stretches
        // AWAY from its bay, covering the drawn tab (the converted geometry
        // spans stand-to-face into the aisle); a slot's is a small square.
        // Poses on the floor (z 0) are already answered by their cells.
        // A board's box hugs the INSIDE of the steel, like the drawn tab:
        // long inward from the face, a sliver outward for the click that
        // lands on the face edge itself.
        const SLOT_HALF_MM: f64 = 300.0;
        const BOARD_OUT_MM: f64 = 150.0;
        const BOARD_IN_MM: f64 = 800.0;
        const BOARD_CROSS_MM: f64 = 320.0;
        let mut shelf_points: Vec<([f64; 4], f64, u32)> = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let EntityPlace::Point([px, py, pz]) = row.place else { continue };
            if pz <= 0.0 {
                continue;
            }
            let is_board = row.components.iter().any(|name| name.as_ref() == "relay");
            let rect = if is_board {
                // The nearest standing bay says which way "outward" is: the
                // exchange point sits on a bay face, and the tab hangs off it.
                let bay = prisms
                    .iter()
                    .filter(|&&(_, _, top, _)| top > 0.0)
                    .map(|&(_, rect, _, _)| rect)
                    .min_by(|a, b| rect_centre_dist(*a, px, py).total_cmp(&rect_centre_dist(*b, px, py)));
                match bay {
                    Some([rx, ry, rw, rh]) => {
                        let (dx, dy) = (px - (rx + rw / 2.0), py - (ry + rh / 2.0));
                        if dx.abs() / rw >= dy.abs() / rh {
                            let (out, inw) =
                                if dx >= 0.0 { (BOARD_OUT_MM, BOARD_IN_MM) } else { (BOARD_IN_MM, BOARD_OUT_MM) };
                            [px - inw, py - BOARD_CROSS_MM, inw + out, BOARD_CROSS_MM * 2.0]
                        } else {
                            let (out, inw) =
                                if dy >= 0.0 { (BOARD_OUT_MM, BOARD_IN_MM) } else { (BOARD_IN_MM, BOARD_OUT_MM) };
                            [px - BOARD_CROSS_MM, py - inw, BOARD_CROSS_MM * 2.0, inw + out]
                        }
                    }
                    None => [px - SLOT_HALF_MM, py - SLOT_HALF_MM, SLOT_HALF_MM * 2.0, SLOT_HALF_MM * 2.0],
                }
            } else {
                [px - SLOT_HALF_MM, py - SLOT_HALF_MM, SLOT_HALF_MM * 2.0, SLOT_HALF_MM * 2.0]
            };
            shelf_points.push((rect, pz, index as u32));
        }

        EntityPanel {
            derived: rows.len() - declared,
            declared,
            chips: chip_counts.into_iter().map(|(name, count)| Chip { name, count, active: false }).collect(),
            rows,
            world,
            groups,
            orphans,
            cell_row,
            prisms,
            shelf_points,
            group_of_root: root_of_declared,
            ..EntityPanel::default()
        }
    }

    /// Take the rows an ⌥-click's ray passed through, nearest first: the
    /// list narrows to exactly these, and the nearest is pinned.
    pub(crate) fn open_ray(&mut self, hits: Vec<u32>) {
        let Some(&first) = hits.first() else { return };
        self.ray_filter = Some(hits);
        self.open_row(first);
    }

    /// Open the panel on one row the map picked: pin it, unfold its parent,
    /// and scroll it into view. If the current filter would hide it, the
    /// filter yields — the click named a specific entity and must land.
    pub(crate) fn open_row(&mut self, row: u32) {
        self.open = true;
        self.pinned = Some(row);
        let row_ref = &self.rows[row as usize];
        if row_ref.derived {
            if let Some(&group) = self.group_of_root.get(derived_root(&row_ref.id_text)) {
                self.expanded.insert(group);
            }
        }
        self.scroll_to = Some(row);
        self.visible_dirty = true;
    }

    /// The map's ONE hover card: the simulator's reading of the cell under
    /// the pointer, then every entity the pick ray passes through, nearest
    /// first. This replaced the old cell tooltip outright — one pointer, one
    /// card.
    pub(crate) fn map_hover_card(&self, ui: &mut egui::Ui, hits: &[u32], cell: Option<&CellInfo>) {
        if let Some(info) = cell {
            ui.monospace(format!("cell {} · {:?} · zone {}", info.id, info.kind, info.zone_id));
            if let Some(relay) = &info.relay_info {
                ui.monospace(relay);
            }
            if let Some(station) = &info.station_info {
                ui.monospace(station);
            }
        }
        const LISTED: usize = 4;
        for (position, &hit) in hits.iter().take(LISTED).enumerate() {
            if position > 0 || cell.is_some() {
                ui.separator();
            }
            let row = &self.rows[hit as usize];
            ui.monospace(&row.id_text);
            if let Some(name) = &row.name {
                ui.monospace(format!("name: {name}"));
            }
            ui.monospace(format!(
                "{} · {}",
                if row.derived { "derived" } else { "declared" },
                row.components.join("+"),
            ));
            ui.monospace(place_summary(&row.place));
        }
        if hits.len() > LISTED {
            ui.label(theme::caption(&format!("…and {} more along the ray", hits.len() - LISTED)));
        }
        ui.label(theme::caption("⌥-click: open these in Entities"));
    }

    /// Whether `row` passes the text filter and every active chip.
    fn passes(&self, row: &Row, needle: &str, active: &[Arc<str>]) -> bool {
        if !active.iter().all(|chip| row.components.iter().any(|name| name.as_ref() == chip.as_ref())) {
            return false;
        }
        if needle.is_empty() {
            return true;
        }
        // `region+tiling` in the box means "carries both", the way the roll
        // used to print component sets.
        if needle.contains('+') {
            return needle
                .split('+')
                .filter(|part| !part.is_empty())
                .all(|part| row.components.iter().any(|name| contains_lower(name, part)));
        }
        contains_lower(&row.id_text, needle)
            || row.name.as_deref().is_some_and(|name| contains_lower(name, needle))
            || row.components.iter().any(|name| contains_lower(name, needle))
    }

    /// Rebuild the flattened tree the body draws. A declared row shows when it
    /// passes or any of its derivation does; its children show when it is
    /// unfolded — or when only they matched, because then they are the result.
    fn rebuild_visible(&mut self) {
        if !self.visible_dirty {
            return;
        }
        self.visible_dirty = false;
        let needle = self.filter.trim().to_ascii_lowercase();
        let active: Vec<Arc<str>> =
            self.chips.iter().filter(|chip| chip.active).map(|chip| Arc::clone(&chip.name)).collect();
        let filtering = !needle.is_empty() || !active.is_empty();

        // An ⌥-click's ray owns the list while it stands: its hits, grouped
        // under their declaring roots — the declared entity rides along as
        // context even when only its derivation was hit, because a derived
        // thing is read through what declared it (and the root often has no
        // geometry of its own for the ray to meet). Groups stand in
        // nearest-hit order; hit children indent under their root in ray
        // order. The text and chip filters narrow the hits.
        if let Some(ray) = &self.ray_filter {
            let mut tops: Vec<u32> = Vec::new();
            let mut hit_children: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
            for &row in ray {
                if !self.passes(&self.rows[row as usize], &needle, &active) {
                    continue;
                }
                let top = if self.rows[row as usize].derived {
                    self.group_of_root
                        .get(derived_root(&self.rows[row as usize].id_text))
                        .map(|&group| self.groups[group as usize].row)
                        // A rootless derived row stands alone as its own top.
                        .unwrap_or(row)
                } else {
                    row
                };
                if !tops.contains(&top) {
                    tops.push(top);
                }
                if row != top {
                    hit_children.entry(top).or_default().push(row);
                }
            }
            let mut visible = Vec::new();
            let mut shown_declared = 0;
            let mut shown_derived = 0;
            for &top in &tops {
                if self.rows[top as usize].derived {
                    shown_derived += 1;
                } else {
                    shown_declared += 1;
                }
                visible.push(VisRow { row: top, kind: VisKind::Flat });
                for &child in hit_children.get(&top).map_or(&[][..], Vec::as_slice) {
                    shown_derived += 1;
                    visible.push(VisRow { row: child, kind: VisKind::Child });
                }
            }
            self.visible = visible;
            self.shown_declared = shown_declared;
            self.shown_derived = shown_derived;
            return;
        }

        let mut visible = Vec::new();
        let mut shown_declared = 0;
        let mut shown_derived = 0;
        for (group_index, group) in self.groups.iter().enumerate() {
            let parent = &self.rows[group.row as usize];
            let parent_pass = self.passes(parent, &needle, &active);
            let total_children = group.children.len();
            let matching: Vec<u32> = if filtering && !parent_pass {
                group
                    .children
                    .clone()
                    .filter(|&child| self.passes(&self.rows[child as usize], &needle, &active))
                    .collect()
            } else {
                Vec::new()
            };
            if !parent_pass && matching.is_empty() {
                continue;
            }
            shown_declared += 1;
            let shown_children = if parent_pass { total_children } else { matching.len() };
            // A parent shown only for its matching derivation unfolds by
            // itself — the children ARE what matched — unless a click folded
            // it shut over the filter.
            let unfolded = (self.expanded.contains(&(group_index as u32)) || (filtering && !parent_pass))
                && !self.collapsed.contains(&(group_index as u32));
            visible.push(VisRow {
                row: group.row,
                kind: VisKind::Parent { group: group_index as u32, shown_children, total_children, unfolded },
            });
            if !unfolded {
                continue;
            }
            if parent_pass {
                for child in group.children.clone() {
                    visible.push(VisRow { row: child, kind: VisKind::Child });
                }
                shown_derived += total_children;
            } else {
                shown_derived += matching.len();
                visible.extend(matching.into_iter().map(|child| VisRow { row: child, kind: VisKind::Child }));
            }
        }
        for &orphan in &self.orphans {
            if self.passes(&self.rows[orphan as usize], &needle, &active) {
                shown_derived += 1;
                visible.push(VisRow { row: orphan, kind: VisKind::Flat });
            }
        }
        self.visible = visible;
        self.shown_declared = shown_declared;
        self.shown_derived = shown_derived;
    }

    /// Open (or close, when it is the one already open) the body of one
    /// component on one row.
    fn toggle_detail(&mut self, row: u32, component: &Arc<str>) {
        if self
            .detail
            .as_ref()
            .is_some_and(|detail| detail.row == row && detail.component.as_ref() == component.as_ref())
        {
            self.detail = None;
            return;
        }
        let body = self.component_body(row, component);
        self.detail = Some(Detail { row, component: Arc::clone(component), body });
    }

    /// The component's body from the retained document, pretty-printed.
    fn component_body(&self, row: u32, component: &str) -> String {
        let Some(world) = &self.world else {
            return "(the document was not retained in this session)".to_string();
        };
        let row = &self.rows[row as usize];
        let Ok(id) = row.id_text.parse() else {
            return "(identity did not parse back — this is a bug)".to_string();
        };
        let Some(entity) = world.entities.get(&id) else {
            return "(entity not found in the retained document — this is a bug)".to_string();
        };
        match entity.components.get(component) {
            Some(value) => serde_json::to_string_pretty(value)
                .unwrap_or_else(|error| format!("(component body did not serialize: {error})")),
            None => "(the entity carries no such component — this is a bug)".to_string(),
        }
    }
}

impl DesvisApp {
    /// Draw the entity explorer and record what the pointer is over.
    ///
    /// Declared before the central panel for two reasons: a docked panel only
    /// reserves its strip of the window if it is declared first, and the row
    /// the pointer lands on decides what the map outlines this same frame.
    pub(crate) fn draw_entity_panel(&mut self, ctx: &egui::Context) {
        self.entity_panel.hovered = None;
        let mut keep_open = self.entity_panel.open;

        // Transparent panel frame with the card painted inside it, the same
        // treatment the left sidebar and the bottom dock get: the card's
        // rounded edge would otherwise be clipped at the panel boundary.
        egui::SidePanel::right("entities")
            .resizable(true)
            .default_width(DEFAULT_WIDTH)
            .width_range(MIN_WIDTH..=MAX_WIDTH)
            .show_separator_line(false)
            .frame(egui::Frame::none())
            .show_animated(ctx, self.entity_panel.open, |ui| {
                theme::card_frame().show(ui, |ui| {
                    // Claim the rect the panel handed us. A side panel keeps
                    // its content rect between frames, so measuring our own
                    // and setting a width from it makes the panel fight the
                    // drag handle.
                    ui.set_min_size(ui.available_size());
                    self.draw_entity_body(ui, &mut keep_open);
                });
            });
        self.entity_panel.open = keep_open;
    }

    /// Header, filter, chips, count line, the tree, then the open component
    /// body docked at the bottom.
    fn draw_entity_body(&mut self, ui: &mut egui::Ui, keep_open: &mut bool) {
        ui.horizontal(|ui| {
            ui.label(theme::heading("Entities"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::button(ui, "×").clicked() {
                    *keep_open = false;
                }
            });
        });
        ui.add_space(4.0);

        if self.entity_panel.rows.is_empty() {
            ui.label(theme::caption("(no world document loaded — the roll comes from the document, not the map)"));
            return;
        }

        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.entity_panel.filter)
                    .desired_width(ui.available_width() - 60.0)
                    .hint_text("filter id / name / component…"),
            );
            if response.changed() {
                self.entity_panel.collapsed.clear();
                self.entity_panel.visible_dirty = true;
            }
            if theme::button(ui, "clear").clicked() {
                self.entity_panel.filter.clear();
                for chip in &mut self.entity_panel.chips {
                    chip.active = false;
                }
                self.entity_panel.collapsed.clear();
                self.entity_panel.visible_dirty = true;
            }
        });
        ui.add_space(4.0);

        // The component bar: one lozenge per distinct name. Active lozenges
        // AND together — the roll narrows to entities carrying all of them.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for chip in &mut self.entity_panel.chips {
                let label = format!("{} {}", chip.name, chip.count);
                if lozenge(ui, &label, chip.active).clicked() {
                    chip.active = !chip.active;
                    self.entity_panel.collapsed.clear();
                    self.entity_panel.visible_dirty = true;
                }
            }
        });
        ui.add_space(2.0);

        // The standing ray filter, if an ⌥-click set one.
        if self.entity_panel.ray_filter.is_some() {
            let count = self.entity_panel.ray_filter.as_ref().map_or(0, Vec::len);
            ui.horizontal(|ui| {
                ui.label(theme::caption(&format!("{count} along the ⌥-click ray, nearest first")));
                if theme::button(ui, "×").clicked() {
                    self.entity_panel.ray_filter = None;
                    self.entity_panel.visible_dirty = true;
                }
            });
            ui.add_space(2.0);
        }

        self.entity_panel.rebuild_visible();
        // A row the map opened must land even when the filter would hide it:
        // the click named a specific entity, so the filter yields.
        if let Some(target) = self.entity_panel.scroll_to {
            if !self.entity_panel.visible.iter().any(|vis| vis.row == target) {
                self.entity_panel.filter.clear();
                for chip in &mut self.entity_panel.chips {
                    chip.active = false;
                }
                self.entity_panel.visible_dirty = true;
                self.entity_panel.rebuild_visible();
            }
        }
        ui.label(theme::caption(&format!(
            "{} declared / {} derived · showing {} / {} · dimmed = nowhere on the map",
            self.entity_panel.declared,
            self.entity_panel.derived,
            self.entity_panel.shown_declared,
            self.entity_panel.shown_derived,
        )));
        ui.add_space(4.0);

        if self.entity_panel.visible.is_empty() {
            ui.label(theme::caption("(no entity matches the filter)"));
            return;
        }

        // The header sits OUTSIDE the scroll area, so it stays put while the
        // body scrolls under it. Both take the same `Columns`, measured from
        // the width available here, before the scroll area narrows anything.
        let columns = Columns::of(ui.available_width(), ui.spacing().item_spacing.x);
        ui.horizontal(|ui| {
            for (width, title) in [(columns.id, "id"), (columns.name, "name"), (columns.components, "components")] {
                cell(ui, width, |ui| {
                    ui.label(theme::heading(title));
                });
            }
        });
        ui.separator();

        // The open component body docks under the tree, so the tree's scroll
        // area is capped to what the body leaves it.
        let detail_height =
            if self.entity_panel.detail.is_some() { (ui.available_height() * 0.45).clamp(120.0, 280.0) } else { 0.0 };

        // The scrollbar floats over the content (see `apply_scrollbar_style`),
        // so it takes no width from the body and the header's columns still
        // line up with the rows'.
        theme::apply_scrollbar_style(ui);
        let total = self.entity_panel.visible.len();
        let mut toggle_group: Option<(u32, bool)> = None;
        let mut toggle_lozenge: Option<(u32, Arc<str>)> = None;
        let mut toggle_pin: Option<u32> = None;
        let mut area = egui::ScrollArea::vertical()
            .max_height((ui.available_height() - detail_height).max(60.0))
            .auto_shrink([false, false]);
        if let Some(position) = self
            .entity_panel
            .scroll_to
            .take()
            .and_then(|target| self.entity_panel.visible.iter().position(|vis| vis.row == target))
        {
            // The pitch `show_rows` scrolls by is the row height PLUS the
            // vertical item spacing it inserts between rows — multiplying by
            // the bare height lands short by the accumulated spacing.
            let row_pitch = ROW_HEIGHT + ui.spacing().item_spacing.y;
            // A little above the target, so it lands in context rather than
            // flush at the top edge.
            area = area.vertical_scroll_offset((position as f32 * row_pitch - 100.0).max(0.0));
        }
        area.show_rows(ui, ROW_HEIGHT, total, |ui, range| {
            for position in range {
                let open_detail = self.entity_panel.detail.as_ref();
                let vis = &self.entity_panel.visible[position];
                let row = &self.entity_panel.rows[vis.row as usize];
                let drawn = draw_row(
                    ui,
                    row,
                    vis,
                    columns,
                    open_detail.filter(|detail| detail.row == vis.row).map(|detail| detail.component.as_ref()),
                    self.entity_panel.pinned == Some(vis.row),
                );
                if drawn.hovered {
                    self.entity_panel.hovered = Some(vis.row);
                }
                if drawn.chevron_clicked {
                    if let VisKind::Parent { group, unfolded, .. } = vis.kind {
                        toggle_group = Some((group, unfolded));
                    }
                } else if let Some(component) = drawn.lozenge_clicked {
                    toggle_lozenge = Some((vis.row, component));
                } else if drawn.row_clicked {
                    toggle_pin = Some(vis.row);
                }
            }
        });
        if let Some((group, unfolded)) = toggle_group {
            // Toggle against what the tree actually showed. Folding an
            // auto-unfolded group records an override the filter respects.
            if unfolded {
                self.entity_panel.expanded.remove(&group);
                self.entity_panel.collapsed.insert(group);
            } else {
                self.entity_panel.collapsed.remove(&group);
                self.entity_panel.expanded.insert(group);
            }
            self.entity_panel.visible_dirty = true;
        }
        if let Some((row, component)) = toggle_lozenge {
            self.entity_panel.toggle_detail(row, &component);
        }
        if let Some(row) = toggle_pin {
            self.entity_panel.pinned = if self.entity_panel.pinned == Some(row) { None } else { Some(row) };
        }

        if self.entity_panel.detail.is_some() {
            self.draw_entity_detail(ui, detail_height);
        }
    }

    /// The open component's body: name, owner, copy, and the pretty JSON.
    fn draw_entity_detail(&mut self, ui: &mut egui::Ui, height: f32) {
        let Some(detail) = &self.entity_panel.detail else { return };
        let owner = self.entity_panel.rows[detail.row as usize].id_text.clone();
        let component = Arc::clone(&detail.component);
        let body = detail.body.clone();

        ui.separator();
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label(theme::heading(&component));
            ui.add(egui::Label::new(theme::caption(&format!("on {owner}"))).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::button(ui, "×").clicked() {
                    close = true;
                }
                if theme::button(ui, "copy").clicked() {
                    ui.output_mut(|out| out.copied_text = body.clone());
                }
            });
        });
        egui::ScrollArea::vertical()
            .id_salt("entity_detail")
            .max_height(height - 30.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // A read-only text edit rather than a label: the body stays
                // selectable, so a value can leave through the clipboard in
                // pieces as well as whole.
                let mut text = body.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY),
                );
            });
        if close {
            self.entity_panel.detail = None;
        }
    }

    /// Outline the hovered entity on the map, in the same projection and with
    /// the same closed-line idiom the click selection uses — one overlay
    /// vocabulary, two reasons to draw in it. Occlusion ignored: this is a
    /// diagnostic outline, not scene geometry.
    ///
    /// Drawn in every view mode. Which entity a rectangle of floor is stays
    /// true whether the map is showing state or a heatmap.
    pub(crate) fn paint_entity_highlight(&self, cam: &CameraState, painter: &egui::Painter, rect: egui::Rect) {
        // Every row of a standing ray filter, faintly: the click selected
        // them all, and the map should say so together.
        if let Some(ray) = &self.entity_panel.ray_filter {
            let faint = egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.45));
            for &member in ray {
                if Some(member) == self.entity_panel.pinned {
                    continue;
                }
                if let Some(row) = self.entity_panel.rows.get(member as usize) {
                    self.paint_entity_place(&row.place, faint, cam, painter, rect);
                }
            }
        }
        // The pinned row first and heavier, the hovered row lighter over it:
        // a pin survives the pointer leaving; a hover is the pointer talking.
        let pinned = self.entity_panel.pinned;
        for (index, stroke) in [
            (pinned, egui::Stroke::new(2.5, theme::ACCENT)),
            (
                self.entity_panel.hovered.filter(|&hovered| Some(hovered) != pinned),
                egui::Stroke::new(1.5, theme::ACCENT.gamma_multiply(0.75)),
            ),
        ] {
            let Some(index) = index else { continue };
            let Some(row) = self.entity_panel.rows.get(index as usize) else { continue };
            self.paint_entity_place(&row.place, stroke, cam, painter, rect);
        }
    }

    /// One entity's place, in one stroke.
    fn paint_entity_place(
        &self,
        place: &EntityPlace,
        stroke: egui::Stroke,
        cam: &CameraState,
        painter: &egui::Painter,
        rect: egui::Rect,
    ) {
        let outline = |corners: [[f64; 2]; 4]| {
            painter.add(egui::Shape::closed_line(
                corners
                    .iter()
                    .map(|&[x, y]| {
                        let [wx, wy] = self.frame.world_mm(x, y);
                        screen::pos(cam.world_to_screen(wx, wy, screen::viewport(rect)))
                    })
                    .collect(),
                stroke,
            ));
        };
        match place {
            EntityPlace::Cells(cells) => {
                for &cell_id in cells {
                    let Some(info) = self.cell_infos.get(cell_id.0 as usize).and_then(|i| i.as_ref()) else {
                        continue;
                    };
                    outline(info.corners_mm());
                }
            }
            EntityPlace::Rect([x, y, w, h]) | EntityPlace::Prism { rect: [x, y, w, h], .. } => {
                outline([[*x, *y], [*x + *w, *y], [*x + *w, *y + *h], [*x, *y + *h]]);
            }
            EntityPlace::Point([x, y, z]) => {
                // Projected at its own height: a slot's ring lands on its
                // shelf under a tilted camera, not on the floor beneath it.
                let [wx, wy] = self.frame.world_mm(*x, *y);
                let at = screen::pos(cam.world_to_screen_z(wx, wy, self.frame.z_of_mm(*z), screen::viewport(rect)));
                painter.circle_stroke(at, 7.0, stroke);
            }
            // Said in the row's hover instead; there is nothing on the floor
            // to ring.
            EntityPlace::Nowhere => {}
        }
    }

    /// Every entity the pick ray passes through: the specific things first —
    /// boards, slots, the cell at the ground crossing — each nearest-first,
    /// then the containers — bays, regions — nearest-first behind them. A
    /// click on a board names the board, not the bay it is inside or the
    /// floor beneath it; the containers still ride along in the list. This
    /// is the map's whole answer to "what is here" — hover reads it, ⌥-click
    /// hands it to the panel.
    pub(crate) fn entity_ray_hits(
        &self,
        ray: Option<([f32; 3], [f32; 3])>,
        ground_mm: Option<[f64; 2]>,
        ground_cell: Option<u32>,
    ) -> Vec<u32> {
        // (is_container, ray_t, row)
        let mut scored: Vec<(bool, f32, u32)> = Vec::new();
        let ground_t = ray.and_then(|(origin, dir)| {
            if dir[2].abs() < 1e-8 {
                return None;
            }
            let t = -origin[2] / dir[2];
            (t >= 0.0).then_some(t)
        });
        if let (Some(t), Some(cell)) = (ground_t, ground_cell) {
            if let Some(&row) = self.entity_panel.cell_row.get(&cell) {
                scored.push((false, t, row));
            }
        }
        if let (Some(t), Some([x, y])) = (ground_t, ground_mm) {
            for &(_, [rx, ry, rw, rh], top, row) in &self.entity_panel.prisms {
                if top <= 0.0 && x >= rx && x <= rx + rw && y >= ry && y <= ry + rh {
                    scored.push((true, t, row));
                }
            }
        }
        if let Some((origin, dir)) = ray {
            let mut hit_bays: Vec<[f64; 4]> = Vec::new();
            for &(_, [rx, ry, rw, rh], top, row) in &self.entity_panel.prisms {
                if top <= 0.0 {
                    continue;
                }
                // Millimetres to render space; the frame flips Y, so take
                // min/max over the converted corners rather than trusting
                // the order.
                let a = self.frame.world_mm(rx, ry);
                let b = self.frame.world_mm(rx + rw, ry + rh);
                let min = [a[0].min(b[0]), a[1].min(b[1]), 0.0];
                let max = [a[0].max(b[0]), a[1].max(b[1]), self.frame.z_of_mm(top)];
                if let Some(t) = ray_box(origin, dir, min, max) {
                    scored.push((true, t, row));
                    hit_bays.push([rx, ry, rw, rh]);
                }
            }
            // Shelved points — slots, boards — only inside bays the ray
            // already met (widened for boards, whose exchange point hangs
            // off the aisle side of the steel). This is what keeps the
            // per-frame cost flat while a site carries a hundred thousand
            // slots.
            const BAY_MARGIN_MM: f64 = 1500.0;
            const GROUND_NEAR_MM: f64 = 2500.0;
            const PICK_Z_MM: f64 = 250.0;
            // Near a hit bay, OR near the ray's ground crossing — the second
            // gate is what lets a steep click straight down onto a shelf
            // work when the ray never enters a bay box from the side.
            for &([px, py, pw, ph], pz, row) in &self.entity_panel.shelf_points {
                let (cx, cy) = (px + pw / 2.0, py + ph / 2.0);
                let near_bay = hit_bays.iter().any(|&[rx, ry, rw, rh]| {
                    cx >= rx - BAY_MARGIN_MM
                        && cx <= rx + rw + BAY_MARGIN_MM
                        && cy >= ry - BAY_MARGIN_MM
                        && cy <= ry + rh + BAY_MARGIN_MM
                });
                let near_ground = ground_mm
                    .is_some_and(|[gx, gy]| (cx - gx).abs() <= GROUND_NEAR_MM && (cy - gy).abs() <= GROUND_NEAR_MM);
                if !near_bay && !near_ground {
                    continue;
                }
                let a = self.frame.world_mm(px, py);
                let b = self.frame.world_mm(px + pw, py + ph);
                let min = [a[0].min(b[0]), a[1].min(b[1]), self.frame.z_of_mm((pz - PICK_Z_MM).max(0.0))];
                let max = [a[0].max(b[0]), a[1].max(b[1]), self.frame.z_of_mm(pz + PICK_Z_MM)];
                if let Some(t) = ray_box(origin, dir, min, max) {
                    scored.push((false, t, row));
                }
            }
        }
        // Specific before containing, nearest first within each: a board
        // outranks the bay it is inside, a cell outranks the region over it,
        // and the containers still follow in the list.
        scored.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.total_cmp(&right.1)));
        let mut hits: Vec<u32> = Vec::new();
        for (_, _, row) in scored {
            if !hits.contains(&row) {
                hits.push(row);
            }
        }
        hits
    }
}

/// Squared distance from a rect's centre to a point, for picking the bay a
/// board's exchange point hangs off.
fn rect_centre_dist([rx, ry, rw, rh]: [f64; 4], px: f64, py: f64) -> f64 {
    let (dx, dy) = (px - (rx + rw / 2.0), py - (ry + rh / 2.0));
    dx * dx + dy * dy
}

/// Slab test: the ray's entry parameter into an axis-aligned box, if it hits.
fn ray_box(origin: [f32; 3], dir: [f32; 3], min: [f32; 3], max: [f32; 3]) -> Option<f32> {
    let (mut enter, mut exit) = (0.0f32, f32::INFINITY);
    for axis in 0..3 {
        if dir[axis].abs() < 1e-9 {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
        } else {
            let inv = 1.0 / dir[axis];
            let (mut near, mut far) = ((min[axis] - origin[axis]) * inv, (max[axis] - origin[axis]) * inv);
            if near > far {
                std::mem::swap(&mut near, &mut far);
            }
            enter = enter.max(near);
            exit = exit.min(far);
            if enter > exit {
                return None;
            }
        }
    }
    Some(enter)
}

/// What one drawn row reported back.
struct DrawnRow {
    hovered: bool,
    chevron_clicked: bool,
    lozenge_clicked: Option<Arc<str>>,
    row_clicked: bool,
}

/// Draw one row of the tree.
fn draw_row(
    ui: &mut egui::Ui,
    row: &Row,
    vis: &VisRow,
    columns: Columns,
    open_component: Option<&str>,
    pinned: bool,
) -> DrawnRow {
    let mut drawn = DrawnRow { hovered: false, chevron_clicked: false, lozenge_clicked: None, row_clicked: false };
    // An entity the document places nowhere is written muted, so a reader
    // scanning the list can see which rows will light nothing up before
    // pointing at them. The count line above the table says what that means.
    let id_color = if matches!(row.place, EntityPlace::Nowhere) { theme::TEXT_2 } else { theme::TEXT_0 };

    // The row's own surface is registered BEFORE its chevron and lozenges, so
    // they sit on top and win their clicks; a click anywhere else on the row
    // is the row's, and pins it. Hover the same way: the surface answers even
    // while an inner widget claims the pointer.
    let row_rect = egui::Rect::from_min_size(
        ui.cursor().min,
        egui::vec2((ui.max_rect().right() - ui.cursor().min.x).max(0.0), ROW_HEIGHT),
    );
    let surface = ui.interact(row_rect, ui.id().with(("entity_row", row.id_text.as_str())), egui::Sense::click());
    if pinned {
        ui.painter().rect_filled(row_rect, 3.0, theme::ACCENT.gamma_multiply(0.14));
    } else if surface.hovered() {
        ui.painter().rect_filled(row_rect, 3.0, theme::ACCENT.gamma_multiply(0.05));
    }

    ui.horizontal(|ui| {
        cell(ui, columns.id, |ui| {
            match vis.kind {
                VisKind::Parent { total_children, unfolded, .. } => {
                    if total_children > 0 {
                        let glyph = if unfolded { "▾" } else { "▸" };
                        // A row-stable id: the virtualized body re-draws a
                        // shifting window of rows, and an auto id would move
                        // to a different row whenever the window slides.
                        let chevron = ui
                            .push_id(("chevron", row.id_text.as_str()), |ui| {
                                ui.add(
                                    egui::Button::new(egui::RichText::new(glyph).small())
                                        .frame(false)
                                        .min_size(egui::vec2(14.0, ROW_HEIGHT - 4.0)),
                                )
                            })
                            .inner;
                        if chevron.clicked() {
                            drawn.chevron_clicked = true;
                        }
                    } else {
                        ui.add_space(14.0);
                    }
                }
                VisKind::Child => ui.add_space(INDENT + 14.0),
                VisKind::Flat => ui.add_space(14.0),
            }
            ui.add(egui::Label::new(egui::RichText::new(&row.short_id).small().monospace().color(id_color)).truncate());
        });
        cell(ui, columns.name, |ui| {
            if let Some(name) = &row.name {
                ui.add(egui::Label::new(theme::caption(name)).truncate());
            } else if let VisKind::Parent { shown_children, total_children, .. } = vis.kind {
                if total_children > 0 && shown_children != total_children {
                    ui.add(egui::Label::new(theme::caption(&format!(
                        "{shown_children} of {total_children} derived match"
                    ))));
                }
            }
        });
        cell(ui, columns.components, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for name in row.components.iter() {
                let active = open_component.is_some_and(|open| open == name.as_ref());
                // Row-stable id, same reason as the chevron's.
                let pill = ui.push_id((row.id_text.as_str(), name.as_ref()), |ui| lozenge(ui, name, active)).inner;
                if pill.clicked() {
                    drawn.lozenge_clicked = Some(Arc::clone(name));
                }
            }
        });
    });

    drawn.row_clicked = surface.clicked();
    // Right-click: the row's clipboard menu. On a clone because both the
    // menu and the hover card below consume the response they hang off.
    surface.clone().context_menu(|ui| {
        if ui.button("Copy entity id").clicked() {
            ui.output_mut(|out| out.copied_text = row.id_text.clone());
            ui.close_menu();
        }
    });
    // `contains_pointer` rather than `hovered` so an inner widget claiming
    // the pointer doesn't drop the row's own outline on the map.
    if !surface.contains_pointer() {
        return drawn;
    }
    surface.on_hover_ui(|ui| {
        ui.monospace(&row.id_text);
        if let Some(name) = &row.name {
            ui.monospace(format!("name: {name}"));
        }
        ui.monospace(format!("components: {}", row.components.join("+")));
        if let VisKind::Parent { total_children, .. } = vis.kind {
            ui.monospace(format!("derives: {total_children} entities"));
        }
        ui.monospace(place_summary(&row.place));
    });
    drawn.hovered = true;
    drawn
}

/// A component name as a small pill. Active means "this is the open one" in a
/// row and "this is filtering" in the bar — the same visual statement: this
/// name is doing something right now.
fn lozenge(ui: &mut egui::Ui, text: &str, active: bool) -> egui::Response {
    let (fill, text_color, stroke) = if active {
        (theme::ACCENT, egui::Color32::WHITE, egui::Stroke::NONE)
    } else {
        (egui::Color32::TRANSPARENT, theme::TEXT_1, egui::Stroke::new(1.0, theme::TEXT_2))
    };
    ui.add(
        egui::Button::new(egui::RichText::new(text).small().color(text_color))
            .fill(fill)
            .stroke(stroke)
            .rounding(egui::Rounding::same(9.0)),
    )
}

/// One column of a row, occupying exactly the width asked for and clipped to
/// it.
///
/// The space is reserved BEFORE the contents draw. Letting a child ui report
/// back how much it used — egui's ordinary behaviour — is what lets a short
/// name pull the next column left and leaves the table ragged down the page.
fn cell(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
    let clip = rect.intersect(ui.clip_rect());
    let mut column =
        ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    column.set_clip_rect(clip);
    add(&mut column);
}

/// The id as the column shows it. A declared identity is a UUID cut to its
/// leading characters; a derived identity is cut to its `/key`, because the
/// root it hangs off is the declared row it sits indented under.
fn short_id(id: &str, derived: bool) -> String {
    if derived {
        match id.split_once('/') {
            Some((_, key)) => format!("/{key}"),
            None => id.to_string(),
        }
    } else {
        let head = id.get(..ID_PREFIX).unwrap_or(id);
        format!("{head}…")
    }
}

/// The root UUID of a derived identity as written: everything before the first
/// `/`. Used only to cut the contiguous derived tail into per-root runs.
fn derived_root(id: &str) -> &str {
    match id.split_once('/') {
        Some((root, _)) => root,
        None => id,
    }
}

/// What the hover says about where the entity is.
fn place_summary(place: &EntityPlace) -> String {
    match place {
        EntityPlace::Cells(cells) if cells.len() == 1 => "place: one cell".to_string(),
        EntityPlace::Cells(cells) => format!("place: {} cells", cells.len()),
        EntityPlace::Rect([_, _, w, h]) => {
            format!("place: {:.2} × {:.2} m", w / 1000.0, h / 1000.0)
        }
        EntityPlace::Prism { rect: [_, _, w, h], top_mm } => {
            format!("place: {:.2} × {:.2} m, {:.2} m tall", w / 1000.0, h / 1000.0, top_mm / 1000.0)
        }
        EntityPlace::Point([x, y, z]) if *z > 0.0 => {
            format!("place: ({x:.0}, {y:.0}) mm, {:.2} m up", z / 1000.0)
        }
        EntityPlace::Point([x, y, _]) => format!("place: point ({x:.0}, {y:.0}) mm"),
        EntityPlace::Nowhere => "place: none — this entity states a relation".to_string(),
    }
}

/// Whether `haystack` contains `needle`, where the caller has already
/// lowercased `needle` in ASCII.
///
/// Folding one side at a time is what keeps a hundred thousand rows out of the
/// allocator on every keystroke. ASCII folding only: a non-ASCII character
/// matches itself and nothing else, which is right for identities and
/// component names and merely strict for a name written with accents.
fn contains_lower(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let (hay, ndl) = (haystack.as_bytes(), needle.as_bytes());
    if ndl.len() > hay.len() {
        return false;
    }
    (0..=hay.len() - ndl.len())
        .any(|start| hay[start..start + ndl.len()].iter().zip(ndl).all(|(a, b)| a.eq_ignore_ascii_case(b)))
}

#[cfg(test)]
mod tests {
    use super::EntityPanel;
    use desgraph::{CellKind, StationType};
    use dessim::worldmap;
    use std::path::Path;

    #[test]
    fn easyloop_order_presentations_keep_the_presentation_color() {
        for fixture in ["minimal_easyloop.json", "walmart_dfw6.json"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sim/dessim/tests/fixtures").join(fixture);
            let loaded = worldmap::load(&path).expect("map loads");
            let mut checked = 0;
            for station in loaded.map.stations.iter().filter(|station| station.station_type == StationType::Pick) {
                let Some(cell) = station.order_presentation_cell else { continue };
                assert_eq!(loaded.map.graph.cell(cell).unwrap().kind, CellKind::Presentation, "{fixture}: {cell:?}");
                checked += 1;
            }
            assert_eq!(checked, if fixture == "minimal_easyloop.json" { 2 } else { 8 });
        }
    }

    #[test]
    fn easyloop_order_presentations_are_directly_pickable_entities() {
        for fixture in ["minimal_easyloop.json", "walmart_dfw6.json"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sim/dessim/tests/fixtures").join(fixture);
            let loaded = worldmap::load(&path).expect("map loads");
            let panel = EntityPanel::from_census(loaded.entities, Some(loaded.world));
            let mut checked = 0;
            for station in loaded.map.stations.iter().filter(|station| station.station_type == StationType::Pick) {
                let Some(cell) = station.order_presentation_cell else { continue };
                let row = panel.cell_row.get(&cell.0).expect("a presentation cell must resolve in Option-click");
                assert_eq!(panel.rows[*row as usize].id_text, loaded.index.cell_entities[&cell].to_string());
                checked += 1;
            }
            assert_eq!(checked, if fixture == "minimal_easyloop.json" { 2 } else { 8 });
        }
    }
}
