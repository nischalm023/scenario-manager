//! Within `crucible-viewer`, `roll` is WHICH CELLS EACH ENTITY STANDS ON, read
//! both ways: the roll a floor arrives with, as the cells under an entity's
//! key and the keys over a cell (spec 03 section 3.6, what the viewer needs).
//!
//! It exists because the two sides of the viewer name different things. A
//! pointer lands on a cell and the person means an entity, so a pick answers
//! keys; the agent names entities and means where they stand, so an outline
//! takes keys and draws cells. A roll holds only the entities that stand on
//! cells (a sector, a rack or a slot stands elsewhere or nowhere), so a key
//! the roll does not hold has no cells here, which callers count rather than
//! refuse. Cells are [`CellId`]s from the wire's numbers on.

use desgraph::CellId;
use rustc_hash::FxHashMap;
use worldview_render::{Census, RollEntry};

/// A roll indexed both ways.
#[derive(Default)]
pub struct Roll {
    entries: Vec<Standing>,
    /// Where each key stands in the roll.
    by_key: FxHashMap<String, usize>,
    /// Each cell's entities, by cell id, as indexes into the roll in the
    /// order a pick answers them.
    over: Vec<Vec<usize>>,
}

/// One entity of the roll, its cells typed.
struct Standing {
    key: String,
    cells: Vec<CellId>,
    is_cell: bool,
}

impl Roll {
    /// Index a floor document's roll. Each cell's entities are put in pick
    /// order here, once per floor, so a pointer's answer is a lookup.
    ///
    /// Pick order is fewest cells first, so the most specific thing on a cell
    /// leads (a station before the floor it stands on), with the cell itself
    /// last whatever its count, as the roll marks it, and keys between equals.
    pub fn new(entries: Vec<RollEntry>) -> Self {
        let entries: Vec<Standing> = entries
            .into_iter()
            .map(|RollEntry { key, cells, is_cell }| Standing {
                key,
                cells: cells.into_iter().map(CellId).collect(),
                is_cell,
            })
            .collect();
        let mut by_key = FxHashMap::default();
        let mut over: Vec<Vec<usize>> = Vec::new();
        for (at, entry) in entries.iter().enumerate() {
            by_key.insert(entry.key.clone(), at);
            for &CellId(cell) in &entry.cells {
                let cell = cell as usize;
                if over.len() <= cell {
                    over.resize_with(cell + 1, Vec::new);
                }
                over[cell].push(at);
            }
        }
        let order = |entry: &Standing| (entry.is_cell, entry.cells.len(), entry.key.clone());
        for entities in &mut over {
            entities.sort_by_cached_key(|&at| order(&entries[at]));
            entities.dedup();
        }
        Self { entries, by_key, over }
    }

    /// A recording's census, whose scenario block does not say which entity
    /// is a cell: there every entity on a cell sorts by its count and key.
    pub fn of_census(census: Census) -> Self {
        Self::new(census.into_iter().map(|(key, cells)| RollEntry { key, cells, is_cell: false }).collect())
    }

    /// The keys of the entities standing on `cell`, in pick order.
    pub fn keys_at(&self, CellId(cell): CellId) -> Vec<String> {
        let entities = self.over.get(cell as usize).map_or(&[][..], Vec::as_slice);
        entities.iter().map(|&at| self.entries[at].key.clone()).collect()
    }

    /// The cells `key` stands on: none for a key the roll does not hold.
    pub fn cells_of(&self, key: &str) -> &[CellId] {
        self.by_key.get(key).map_or(&[], |&at| &self.entries[at].cells)
    }

    /// Every cell the keys stand on, each once and in id order, and how many
    /// of the keys (each counted once) stand on any.
    pub fn cells_of_all<'a>(&self, keys: impl IntoIterator<Item = &'a str>) -> (Vec<CellId>, u32) {
        let mut named: Vec<&str> = keys.into_iter().collect();
        named.sort_unstable();
        named.dedup();
        let mut cells = Vec::new();
        let mut standing = 0;
        for key in named {
            let of = self.cells_of(key);
            standing += u32::from(!of.is_empty());
            cells.extend_from_slice(of);
        }
        cells.sort_unstable();
        cells.dedup();
        (cells, standing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, cells: &[u32], is_cell: bool) -> RollEntry {
        RollEntry { key: key.into(), cells: cells.to_vec(), is_cell }
    }

    fn roll() -> Roll {
        Roll::new(vec![
            entry("cell-b", &[1], true),
            entry("station", &[0, 1, 2], false),
            entry("queue", &[1, 2], false),
            entry("cell-a", &[0], true),
        ])
    }

    /// Spec 03 section 3.6: the keys over a cell come fewest cells first and
    /// the cell itself last, so a station is answered before the cell it
    /// stands on.
    #[test]
    fn a_cell_answers_the_fewest_cells_first_and_itself_last() {
        let roll = roll();
        assert_eq!(roll.keys_at(CellId(1)), ["queue", "station", "cell-b"]);
        assert_eq!(roll.keys_at(CellId(0)), ["station", "cell-a"]);
        assert!(roll.keys_at(CellId(7)).is_empty(), "a cell nothing stands on answers nothing");
    }

    /// A station of one cell counts as many cells as the cell it stands on,
    /// and its key sorts after the cell's here: the roll's own mark, not the
    /// count or the key, puts the cell itself last.
    #[test]
    fn a_one_cell_station_is_answered_before_the_cell_it_stands_on() {
        let roll = Roll::new(vec![entry("a-cell", &[4], true), entry("z-station", &[4], false)]);
        assert_eq!(roll.keys_at(CellId(4)), ["z-station", "a-cell"]);
        let unmarked = Roll::of_census(vec![("a-cell".into(), vec![4]), ("z-station".into(), vec![4])]);
        let keys = unmarked.keys_at(CellId(4));
        assert_eq!(keys, ["a-cell", "z-station"], "a recording's census marks no cell, so keys decide");
    }

    #[test]
    fn keys_answer_their_cells_once_each_and_count_the_keys_standing_anywhere() {
        let roll = roll();
        let (cells, standing) = roll.cells_of_all(["queue", "station", "a sector", "queue"]);
        assert_eq!(cells, [CellId(0), CellId(1), CellId(2)]);
        assert_eq!(standing, 2, "the sector stands on no cell, and a key named twice is one key");
        assert!(roll.cells_of("a sector").is_empty());
    }
}
