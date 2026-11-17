//! Empirical comparison of data structure variants for InventoryService.
//!
//! Tests three approaches for the hot paths:
//! - Variant A: Current (BTreeMap/BTreeSet)
//! - Variant B: HashMap + flat Vec
//! - Variant C: Pre-materialized indexes (empty totes grouped by grid region)
//!
//! Each variant implements the same operations at cencora scale (9,600 totes).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Variant A: BTreeMap/BTreeSet (current implementation)
// ---------------------------------------------------------------------------

mod variant_a {
    use std::collections::{BTreeMap, BTreeSet};

    pub struct State {
        positions: Vec<(u32, u32)>, // flat Vec, indexed by id
        empty_in_rack: BTreeSet<u32>,
        sku_totes: BTreeMap<u32, BTreeSet<u32>>, // sku → tote ids
        stock_qty: BTreeMap<(u32, u32), u32>,    // (sku, tote) → qty
    }

    impl State {
        pub fn new(n: u32) -> Self {
            let mut positions = vec![(u32::MAX, u32::MAX); n as usize];
            let mut empty_in_rack = BTreeSet::new();
            for i in 0..n {
                positions[i as usize] = ((i * 7 + 13) % 200, (i * 3 + 7) % 100);
                empty_in_rack.insert(i);
            }
            Self { positions, empty_in_rack, sku_totes: BTreeMap::new(), stock_qty: BTreeMap::new() }
        }

        pub fn add_item(&mut self, tote: u32, sku: u32, qty: u32) {
            self.empty_in_rack.remove(&tote);
            *self.stock_qty.entry((sku, tote)).or_insert(0) += qty;
            self.sku_totes.entry(sku).or_default().insert(tote);
        }

        pub fn find_nearest_empty(&self, near: (u32, u32), max: usize) -> Vec<(u32, u32)> {
            let mut scored: Vec<(u32, u32)> = self
                .empty_in_rack
                .iter()
                .filter_map(|&id| {
                    let pos = self.positions[id as usize];
                    if pos == (u32::MAX, u32::MAX) {
                        return None;
                    }
                    Some((near.0.abs_diff(pos.0) + near.1.abs_diff(pos.1), id))
                })
                .collect();
            let k = max.min(scored.len());
            if k == 0 {
                return vec![];
            }
            if scored.len() > k {
                scored.select_nth_unstable_by_key(k, |s| s.0);
                scored.truncate(k);
            }
            scored.sort_unstable_by_key(|s| s.0);
            scored
        }

        pub fn find_nearest_sku(&self, sku: u32, near: (u32, u32), max: usize) -> Vec<(u32, u32)> {
            let totes = match self.sku_totes.get(&sku) {
                Some(t) => t,
                None => return vec![],
            };
            let mut scored: Vec<(u32, u32)> = totes
                .iter()
                .filter_map(|&id| {
                    let pos = self.positions[id as usize];
                    if pos == (u32::MAX, u32::MAX) {
                        return None;
                    }
                    Some((near.0.abs_diff(pos.0) + near.1.abs_diff(pos.1), id))
                })
                .collect();
            let k = max.min(scored.len());
            if k == 0 {
                return vec![];
            }
            if scored.len() > k {
                scored.select_nth_unstable_by_key(k, |s| s.0);
                scored.truncate(k);
            }
            scored.sort_unstable_by_key(|s| s.0);
            scored
        }
    }
}

// ---------------------------------------------------------------------------
// Variant B: HashMap + flat Vec
// ---------------------------------------------------------------------------

mod variant_b {
    use std::collections::HashMap;

    pub struct State {
        positions: Vec<(u32, u32)>,
        empty_in_rack: Vec<u32>,                  // flat unsorted
        sku_stock: HashMap<u32, Vec<(u32, u32)>>, // sku → vec of (tote, qty)
    }

    impl State {
        pub fn new(n: u32) -> Self {
            let mut positions = vec![(u32::MAX, u32::MAX); n as usize];
            let mut empty_in_rack = Vec::with_capacity(n as usize);
            for i in 0..n {
                positions[i as usize] = ((i * 7 + 13) % 200, (i * 3 + 7) % 100);
                empty_in_rack.push(i);
            }
            Self { positions, empty_in_rack, sku_stock: HashMap::new() }
        }

        pub fn add_item(&mut self, tote: u32, sku: u32, qty: u32) {
            if let Some(idx) = self.empty_in_rack.iter().position(|&t| t == tote) {
                self.empty_in_rack.swap_remove(idx);
            }
            let entries = self.sku_stock.entry(sku).or_default();
            if let Some(e) = entries.iter_mut().find(|e| e.0 == tote) {
                e.1 += qty;
            } else {
                entries.push((tote, qty));
            }
        }

        pub fn find_nearest_empty(&self, near: (u32, u32), max: usize) -> Vec<(u32, u32)> {
            let mut scored: Vec<(u32, u32)> = self
                .empty_in_rack
                .iter()
                .map(|&id| {
                    let pos = self.positions[id as usize];
                    (near.0.abs_diff(pos.0) + near.1.abs_diff(pos.1), id)
                })
                .collect();
            let k = max.min(scored.len());
            if k == 0 {
                return vec![];
            }
            if scored.len() > k {
                scored.select_nth_unstable_by_key(k, |s| s.0);
                scored.truncate(k);
            }
            scored.sort_unstable_by_key(|s| s.0);
            scored
        }

        pub fn find_nearest_sku(&self, sku: u32, near: (u32, u32), max: usize) -> Vec<(u32, u32)> {
            let entries = match self.sku_stock.get(&sku) {
                Some(e) => e,
                None => return vec![],
            };
            let mut scored: Vec<(u32, u32)> = entries
                .iter()
                .map(|&(id, _qty)| {
                    let pos = self.positions[id as usize];
                    (near.0.abs_diff(pos.0) + near.1.abs_diff(pos.1), id)
                })
                .collect();
            let k = max.min(scored.len());
            if k == 0 {
                return vec![];
            }
            if scored.len() > k {
                scored.select_nth_unstable_by_key(k, |s| s.0);
                scored.truncate(k);
            }
            scored.sort_unstable_by_key(|s| s.0);
            scored
        }
    }
}

// ---------------------------------------------------------------------------
// Variant C: Pre-materialized spatial grid for empty totes
// ---------------------------------------------------------------------------

mod variant_c {
    use std::collections::HashMap;

    const GRID_SIZE: u32 = 20; // 200/20 = 10 columns, 100/20 = 5 rows = 50 cells

    fn grid_cell(x: u32, y: u32) -> usize {
        let col = (x / GRID_SIZE) as usize;
        let row = (y / GRID_SIZE) as usize;
        row * 10 + col // 10 columns
    }

    pub struct State {
        positions: Vec<(u32, u32)>,
        /// Empty totes grouped by grid cell. Each cell covers 20×20 grid units.
        empty_grid: Vec<Vec<u32>>, // 50 cells, each a flat Vec of tote IDs
        sku_stock: HashMap<u32, Vec<(u32, u32)>>,
    }

    impl State {
        pub fn new(n: u32) -> Self {
            let mut positions = vec![(u32::MAX, u32::MAX); n as usize];
            let mut empty_grid = vec![Vec::new(); 50];
            for i in 0..n {
                let pos = ((i * 7 + 13) % 200, (i * 3 + 7) % 100);
                positions[i as usize] = pos;
                empty_grid[grid_cell(pos.0, pos.1)].push(i);
            }
            Self { positions, empty_grid, sku_stock: HashMap::new() }
        }

        pub fn add_item(&mut self, tote: u32, sku: u32, qty: u32) {
            let pos = self.positions[tote as usize];
            let cell = grid_cell(pos.0, pos.1);
            if let Some(idx) = self.empty_grid[cell].iter().position(|&t| t == tote) {
                self.empty_grid[cell].swap_remove(idx);
            }
            let entries = self.sku_stock.entry(sku).or_default();
            if let Some(e) = entries.iter_mut().find(|e| e.0 == tote) {
                e.1 += qty;
            } else {
                entries.push((tote, qty));
            }
        }

        pub fn find_nearest_empty(&self, near: (u32, u32), max: usize) -> Vec<(u32, u32)> {
            // Expand from target grid cell outward until we have enough candidates
            let center_col = (near.0 / GRID_SIZE) as i32;
            let center_row = (near.1 / GRID_SIZE) as i32;

            let mut scored: Vec<(u32, u32)> = Vec::with_capacity(max * 4);

            // Search expanding rings: radius 0, 1, 2, ...
            for radius in 0i32..6 {
                for dr in -radius..=radius {
                    for dc in -radius..=radius {
                        if dr.abs() != radius && dc.abs() != radius {
                            continue;
                        } // only ring border
                        let r = center_row + dr;
                        let c = center_col + dc;
                        if r < 0 || r >= 5 || c < 0 || c >= 10 {
                            continue;
                        }
                        let cell_idx = r as usize * 10 + c as usize;
                        for &id in &self.empty_grid[cell_idx] {
                            let pos = self.positions[id as usize];
                            let dist = near.0.abs_diff(pos.0) + near.1.abs_diff(pos.1);
                            scored.push((dist, id));
                        }
                    }
                }
                // Early exit: if we have enough AND next ring can't beat current worst
                if scored.len() >= max {
                    scored.sort_unstable_by_key(|s| s.0);
                    scored.truncate(max);
                    let worst = scored.last().unwrap().0;
                    // Next ring is at least (radius+1)*GRID_SIZE away
                    if (radius as u32 + 1) * GRID_SIZE > worst {
                        return scored;
                    }
                }
            }

            let k = max.min(scored.len());
            if k == 0 {
                return vec![];
            }
            if scored.len() > k {
                scored.select_nth_unstable_by_key(k, |s| s.0);
                scored.truncate(k);
            }
            scored.sort_unstable_by_key(|s| s.0);
            scored
        }

        // SKU search is same as variant_b
        pub fn find_nearest_sku(&self, sku: u32, near: (u32, u32), max: usize) -> Vec<(u32, u32)> {
            let entries = match self.sku_stock.get(&sku) {
                Some(e) => e,
                None => return vec![],
            };
            let mut scored: Vec<(u32, u32)> = entries
                .iter()
                .map(|&(id, _qty)| {
                    let pos = self.positions[id as usize];
                    (near.0.abs_diff(pos.0) + near.1.abs_diff(pos.1), id)
                })
                .collect();
            let k = max.min(scored.len());
            if k == 0 {
                return vec![];
            }
            if scored.len() > k {
                scored.select_nth_unstable_by_key(k, |s| s.0);
                scored.truncate(k);
            }
            scored.sort_unstable_by_key(|s| s.0);
            scored
        }
    }
}

// ---------------------------------------------------------------------------
// Benchmark harness
// ---------------------------------------------------------------------------

pub fn run_all_benchmarks() {
    let n = 9_600u32;
    let iterations = 100_000u32;
    let near = (100u32, 50u32);

    // Setup: populate ~1000 totes with items, leaving ~8600 empty
    eprintln!("=== InventoryService Data Structure Comparison ===");
    eprintln!("    {n} totes, {iterations} iterations each, target=({}, {})", near.0, near.1);
    eprintln!();

    // --- Variant A ---
    {
        let mut state = variant_a::State::new(n); // keep-qualified: three State types, A/B/C bakeoff
        for i in 0u32..100 {
            state.add_item(i * 96, 10, 3);
        }
        for i in 0u32..1000 {
            state.add_item(i * 9, 100 + i % 50, 2);
        }

        let start = Instant::now();
        for _ in 0..iterations {
            let _ = state.find_nearest_empty(near, 10);
        }
        let empty_t = start.elapsed();

        let start = Instant::now();
        for _ in 0..iterations {
            let _ = state.find_nearest_sku(10, near, 10);
        }
        let sku_t = start.elapsed();

        let start = Instant::now();
        for _ in 0..iterations {
            state.add_item(0, 10, 1);
        }
        let mut_t = start.elapsed();

        eprintln!("  Variant A (BTreeMap/BTreeSet):");
        eprintln!("    find_nearest_empty:  {:?}/call", empty_t / iterations);
        eprintln!("    find_nearest_sku:    {:?}/call", sku_t / iterations);
        eprintln!("    add_item (mutation):  {:?}/call", mut_t / iterations);
    }

    // --- Variant B ---
    {
        let mut state = variant_b::State::new(n); // keep-qualified: three State types, A/B/C bakeoff
        for i in 0u32..100 {
            state.add_item(i * 96, 10, 3);
        }
        for i in 0u32..1000 {
            state.add_item(i * 9, 100 + i % 50, 2);
        }

        let start = Instant::now();
        for _ in 0..iterations {
            let _ = state.find_nearest_empty(near, 10);
        }
        let empty_t = start.elapsed();

        let start = Instant::now();
        for _ in 0..iterations {
            let _ = state.find_nearest_sku(10, near, 10);
        }
        let sku_t = start.elapsed();

        let start = Instant::now();
        for _ in 0..iterations {
            state.add_item(0, 10, 1);
        }
        let mut_t = start.elapsed();

        eprintln!("  Variant B (HashMap + flat Vec):");
        eprintln!("    find_nearest_empty:  {:?}/call", empty_t / iterations);
        eprintln!("    find_nearest_sku:    {:?}/call", sku_t / iterations);
        eprintln!("    add_item (mutation):  {:?}/call", mut_t / iterations);
    }

    // --- Variant C ---
    {
        let mut state = variant_c::State::new(n); // keep-qualified: three State types, A/B/C bakeoff
        for i in 0u32..100 {
            state.add_item(i * 96, 10, 3);
        }
        for i in 0u32..1000 {
            state.add_item(i * 9, 100 + i % 50, 2);
        }

        let start = Instant::now();
        for _ in 0..iterations {
            let _ = state.find_nearest_empty(near, 10);
        }
        let empty_t = start.elapsed();

        let start = Instant::now();
        for _ in 0..iterations {
            let _ = state.find_nearest_sku(10, near, 10);
        }
        let sku_t = start.elapsed();

        let start = Instant::now();
        for _ in 0..iterations {
            state.add_item(0, 10, 1);
        }
        let mut_t = start.elapsed();

        eprintln!("  Variant C (spatial grid + HashMap):");
        eprintln!("    find_nearest_empty:  {:?}/call", empty_t / iterations);
        eprintln!("    find_nearest_sku:    {:?}/call", sku_t / iterations);
        eprintln!("    add_item (mutation):  {:?}/call", mut_t / iterations);
    }

    // --- Steady-state mixed workload ---
    eprintln!();
    eprintln!("  Mixed workload (500 empty totes, interleaved reads+writes):");
    {
        let mut state_b = variant_b::State::new(n); // keep-qualified: three State types, A/B/C bakeoff
        let mut state_c = variant_c::State::new(n); // keep-qualified: three State types, A/B/C bakeoff
        // Fill most totes so only ~500 are empty
        for i in 0u32..9100 {
            state_b.add_item(i, 100 + i % 200, 1);
            state_c.add_item(i, 100 + i % 200, 1);
        }

        let mixed_iters = 100_000u32;

        // Variant B mixed
        let start = Instant::now();
        for i in 0..mixed_iters {
            if i % 10 == 0 {
                // mutation every 10th iteration
                state_b.add_item((i * 7) % n, 10, 1);
            } else {
                let _ = state_b.find_nearest_empty(near, 10);
            }
        }
        let b_mixed = start.elapsed();

        // Variant C mixed
        let start = Instant::now();
        for i in 0..mixed_iters {
            if i % 10 == 0 {
                state_c.add_item((i * 7) % n, 10, 1);
            } else {
                let _ = state_c.find_nearest_empty(near, 10);
            }
        }
        let c_mixed = start.elapsed();

        eprintln!("    Variant B mixed:  {:?}/op", b_mixed / mixed_iters);
        eprintln!("    Variant C mixed:  {:?}/op", c_mixed / mixed_iters);
    }
}
