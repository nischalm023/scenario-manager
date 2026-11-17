//! Map topology info and scenario display.

use desgraph::CellKind;
use desmap::{WarehouseMap, Zone};
use dessim::model::{RackId, SkuId, StorageSpaceId};
use dessim::scenario_gen::{Racking, ScenarioData};
use dessim::{ScenarioConfig, SimConfig};
use std::collections::{BTreeMap, BTreeSet};

/// Print map topology, fleet, and scenario summary.
pub(crate) fn run_info(map_path: &str, verbose: bool) {
    let loaded = crate::world_cli::load(map_path);
    let racking = loaded.racking();
    let scenario_config = loaded.scenario.clone();
    let map = loaded.world.map.clone();

    // -- Grid --
    if let Some(bounds) = &map.grid_bounds {
        println!("Grid:          {}x{} ({} cells total)", bounds.width, bounds.depth, bounds.width * bounds.depth);
        println!("Cell size:     {} mm", bounds.cell_size_mm);
    }

    // -- Cell kinds --
    let mut kind_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total_cells = 0usize;
    for cell in map.graph.cells() {
        let label = format!("{:?}", cell.kind);
        *kind_counts.entry(label).or_default() += 1;
        total_cells += 1;
    }
    println!();
    println!("Cells by kind:");
    for (kind, count) in &kind_counts {
        println!("  {:<14} {}", kind, count);
    }
    println!("  {:<14} {}", "TOTAL", total_cells);

    // -- Stations --
    let total_bins: usize = map.stations.iter().map(|s| s.bins.unwrap_or(1)).sum();
    println!();
    println!("Stations:      {} ({} bins total)", map.stations.len(), total_bins);
    for s in &map.stations {
        let bins = s.bins.unwrap_or(1);
        let pres: Vec<String> = s.presentation_cells().map(|cell| format!("{cell:?}")).collect();
        println!("  Station {:>2}   pos={:?}  pres={}  bins={}", s.id.0, s.position, pres.join(", "), bins);
    }

    // -- Relay positions --
    println!();
    println!("Relay positions: {}", map.relays.len());
    if verbose {
        for r in &map.relays {
            println!("  Relay {:>2}  htm_access={:?}  vtm_access={:?}", r.id.0, r.htm_access, r.vtm_access);
        }
    }

    // -- Vertical tote capacity --
    // The stated racking says how high the shelves stack; v3 read this
    // back out of the design's relay zones.
    let vtc = racking.vertical_tote_capacity;

    // -- Zones (only relay zones, skip catch-all) --
    let relay_zones: Vec<_> = map.zones.iter().filter(|z| !z.relays.is_empty()).collect();
    println!();
    println!("Relay zones:   {} (of {} total zones)", relay_zones.len(), map.zones.len());
    // Storage rack entities per zone (mirrors `topology::build_rack_layout`):
    // one per grid rack cell by default; with explicit `slots` every storage
    // slot is its own rack.
    let storage_columns = |z: &Zone| -> usize {
        match &z.slots {
            Some(slots) => slots.iter().filter(|s| !s.diving_board).count(),
            None => z
                .cells
                .iter()
                .filter(|&&cid| map.graph.cell(cid).map(|c| c.kind == CellKind::Relay).unwrap_or(false))
                .count(),
        }
    };
    for z in &relay_zones {
        let name = z.name.as_deref().unwrap_or("(unnamed)");
        let rack_cells = z
            .cells
            .iter()
            .filter(|&&cid| map.graph.cell(cid).map(|c| c.kind == CellKind::Relay).unwrap_or(false))
            .count();
        let relay_positions = z.relays.len();
        let aisle_cells = z.cells.len() - rack_cells;
        let columns = storage_columns(z);
        let storage_slots = columns as u32 * vtc;
        let slots_note = z
            .slots
            .as_ref()
            .map(|slots| {
                let boards = slots.iter().filter(|s| s.diving_board).count();
                format!("  (explicit; {} diving-board slots)", boards)
            })
            .unwrap_or_default();
        println!(
            "  Zone {:>2}  {}  rack_cells={}  relays={}  aisle={}  slots={}{}",
            z.id.0, name, rack_cells, relay_positions, aisle_cells, storage_slots, slots_note
        );
    }
    let total_rack_cells: usize = relay_zones
        .iter()
        .map(|z| {
            z.cells
                .iter()
                .filter(|&&cid| map.graph.cell(cid).map(|c| c.kind == CellKind::Relay).unwrap_or(false))
                .count()
        })
        .sum();
    let total_relay_positions: usize = relay_zones.iter().map(|z| z.relays.len()).sum();
    let total_columns: usize = relay_zones.iter().map(|z| storage_columns(z)).sum();
    let total_storage_slots = total_columns as u32 * vtc;
    println!("  ──────────────────────────────────────────────");
    println!("  Total rack cells:      {}", total_rack_cells);
    println!("  Total relay positions: {}", total_relay_positions);
    println!("  Total storage columns: {}", total_columns);
    println!("  Total storage slots:   {} ({}xvtc {})", total_storage_slots, total_columns, vtc);

    // -- Fleet --
    // VTMs are map-derived (per-zone vtm_count); HTMs/RTPs resolve through
    // the same fleet-block precedence as the launch paths: an explicit
    // non-zero num_htms in the spec WINS over the document's robot count
    // (world_spec::load's rule). Applying the document unconditionally here
    // printed a D30 count the run would not use.
    let mut config = loaded.settings.clone();
    if config.num_htms == 0 {
        dessim::apply_fleet(&mut config, Some(&map.fleet_config()));
    }
    let num_vtms = map.vtm_count();
    println!();
    if num_vtms > 0 || config.num_htms > 0 {
        println!("Fleet:         {} DA50s + {} D30s", num_vtms, config.num_htms);
    } else if config.num_rtps > 0 {
        println!("Fleet:         {} RTPs", config.num_rtps);
    } else {
        println!("Fleet:         (none configured)");
    }

    // -- Zone physical dimensions + VTM task time estimate --
    {
        use dessim::robots::vtm::motion;
        let cell_size_m = map.grid_bounds.as_ref().map(|b| b.cell_size_mm / 1000.0).unwrap_or(1.0);

        println!();
        println!("Zone physical dimensions:");
        for z in &relay_zones {
            let h_mm = z.zone_height_mm;
            let rack_cells = z
                .cells
                .iter()
                .filter(|&&cid| map.graph.cell(cid).map(|c| c.kind == CellKind::Relay).unwrap_or(false))
                .count();
            let derived_w = ((rack_cells as f64 / 2.0) * cell_size_m * 1000.0) as u64;
            let (w_mm, source_w) = match z.zone_width_mm {
                Some(w) => (w, "json"),
                None => (derived_w, "derived"),
            };
            let levels_note = z
                .slots
                .as_ref()
                .map(|slots| {
                    let storage = slots.iter().filter(|s| !s.diving_board).count();
                    let boards = slots.iter().filter(|s| s.diving_board).count();
                    let top = slots.iter().map(|s| s.y).fold(0.0f64, f64::max);
                    format!("  slots={} (+{} diving board, top {:.0}mm)", storage, boards, top)
                })
                .unwrap_or_default();
            println!(
                "  Zone {:>2}: height={}mm  width={}mm ({})  vtc={}{}",
                z.id.0, h_mm, w_mm, source_w, vtc, levels_note
            );
        }

        use motion::{Directionality, Locality, TaskTimeResult};
        let n = 10000;
        let configs: &[(&str, Directionality, Locality)] = &[
            ("Uni/random", Directionality::Unidirectional, Locality::FullRandom),
            ("Uni/walk", Directionality::Unidirectional, Locality::RandomWalk { walk_cells: 3 }),
            ("Bi/random", Directionality::Bidirectional, Locality::FullRandom),
            ("Bi/walk", Directionality::Bidirectional, Locality::RandomWalk { walk_cells: 3 }),
        ];
        let fill_levels: &[f64] = &[0.2, 0.5, 0.8, 1.0];
        let total_vtms = map.vtm_count();
        // This diagnostic accepts only legacy CycleTimes. Catalog VTMs can
        // have floor or ladder chassis and distinct load profiles, so this
        // reference estimate must not claim to model their physical cycles.
        let cycle = config.vtm.cycle_times();

        // Group zones by geometry to avoid redundant output.
        // Key: (height_mm, width_mm, real_shelf_count).
        let mut geom_groups: BTreeMap<(u64, u64, u16), Vec<&Zone>> = BTreeMap::new();
        for z in &relay_zones {
            let rack_cells = z
                .cells
                .iter()
                .filter(|&&cid| map.graph.cell(cid).map(|c| c.kind == CellKind::Relay).unwrap_or(false))
                .count();
            let w_mm = z.zone_width_mm.unwrap_or(((rack_cells as f64 / 2.0) * cell_size_m * 1000.0) as u64);
            geom_groups.entry((z.zone_height_mm, w_mm, z.real_shelf_count)).or_default().push(z);
        }

        println!();
        println!(
            "VTM reference estimate (legacy settings; catalog models not used; synthetic workload, n={}, {} VTMs across {} zones):",
            n,
            total_vtms,
            relay_zones.len()
        );

        for ((h_mm, w_mm, shelves), zones) in &geom_groups {
            let h_m = *h_mm as f64 / 1000.0;
            let w_m = *w_mm as f64 / 1000.0;
            // Physical cell pitch: zone_width / rack_cell_count.
            let rack_cells_in_group = zones[0]
                .cells
                .iter()
                .filter(|&&cid| map.graph.cell(cid).map(|c| c.kind == CellKind::Relay).unwrap_or(false))
                .count();
            let phys_cell_m = if rack_cells_in_group > 0 { w_m / rack_cells_in_group as f64 } else { cell_size_m };
            let zone_vtms: u32 = zones.iter().map(|z| z.vtms_deployed()).sum();
            let zone_ids: Vec<u32> = zones.iter().map(|z| z.id.0).collect();
            if zones.len() == relay_zones.len() {
                println!(
                    "  All {} zones ({:.0}m aisle × {:.1}m height, {} shelves, {} VTMs each):",
                    zones.len(),
                    w_m,
                    h_m,
                    shelves,
                    zones[0].vtms_deployed()
                );
            } else {
                println!(
                    "  Zones {:?} ({:.0}m × {:.1}m, {} shelves, {} VTMs total):",
                    zone_ids, w_m, h_m, shelves, zone_vtms
                );
            }

            // Precompute all cells: task time rows + travel% row for Bi/walk.
            let mut cells: Vec<Vec<String>> = Vec::new();
            for (label, dir, loc) in configs {
                let row: Vec<String> = fill_levels
                    .iter()
                    .map(|&f| {
                        let r = motion::estimate_task_time(w_m, phys_cell_m, h_m, *shelves, *dir, *loc, f, n, cycle);
                        format!("{:.1}s", r.avg_secs)
                    })
                    .collect();
                cells.push(std::iter::once(label.to_string()).chain(row).collect());
            }
            // Time breakdown for Bi/walk (most realistic).
            // Compute once per fill level and reuse.
            let bi_walk_results: Vec<TaskTimeResult> = fill_levels
                .iter()
                .map(|&f| {
                    motion::estimate_task_time(
                        w_m,
                        phys_cell_m,
                        h_m,
                        *shelves,
                        Directionality::Bidirectional,
                        Locality::RandomWalk { walk_cells: 3 },
                        f,
                        n,
                        cycle,
                    )
                })
                .collect();
            let travel_row: Vec<String> = std::iter::once("  travel".to_string())
                .chain(
                    bi_walk_results
                        .iter()
                        .map(|r| format!("{:.1}s ({:.0}%)", r.avg_travel_secs, r.avg_travel_secs / r.avg_secs * 100.0)),
                )
                .collect();
            cells.push(travel_row);
            let alignment_row: Vec<String> = std::iter::once("  alignment".to_string())
                .chain(bi_walk_results.iter().map(|r| {
                    format!("{:.1}s ({:.0}%)", r.avg_alignment_secs, r.avg_alignment_secs / r.avg_secs * 100.0)
                }))
                .collect();
            cells.push(alignment_row);
            let fork_row: Vec<String> = std::iter::once("  fork".to_string())
                .chain(
                    bi_walk_results
                        .iter()
                        .map(|r| format!("{:.1}s ({:.0}%)", r.avg_fork_secs, r.avg_fork_secs / r.avg_secs * 100.0)),
                )
                .collect();
            cells.push(fork_row);
            let buffer_row: Vec<String> = std::iter::once("  relay transfer".to_string())
                .chain(bi_walk_results.iter().map(|r| {
                    format!(
                        "{:.1}s ({:.0}%)",
                        r.avg_relay_transfer_secs,
                        r.avg_relay_transfer_secs / r.avg_secs * 100.0
                    )
                }))
                .collect();
            cells.push(buffer_row);
            // Fleet throughput row.
            let fleet_row: Vec<String> = std::iter::once("fleet t/hr".to_string())
                .chain(bi_walk_results.iter().map(|r| format!("{:.0}", zone_vtms as f64 * 3600.0 / r.avg_secs)))
                .collect();
            cells.push(fleet_row);

            // Column headers.
            let headers: Vec<String> = std::iter::once("Variant".to_string())
                .chain(fill_levels.iter().map(|f| format!("{:.0}% fill", f * 100.0)))
                .collect();

            // Compute column widths.
            let ncols = headers.len();
            let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
            for row in &cells {
                for (c, val) in row.iter().enumerate() {
                    if c < ncols {
                        widths[c] = widths[c].max(val.len());
                    }
                }
            }

            // Print table.
            let sep: String = widths.iter().map(|w| "─".repeat(w + 2)).collect::<Vec<_>>().join("┼");
            let print_row = |vals: &[String]| {
                for (c, val) in vals.iter().enumerate() {
                    if c == 0 {
                        print!("    │ {:<w$} ", val, w = widths[c]);
                    } else {
                        print!("│ {:>w$} ", val, w = widths[c]);
                    }
                }
                println!("│");
            };

            let summary_start = configs.len(); // rows after the variant rows are summary
            println!("    ┌{}┐", sep);
            print_row(&headers);
            println!("    ├{}┤", sep);
            for (i, row) in cells.iter().enumerate() {
                if i == summary_start {
                    println!("    ├{}┤", sep);
                }
                print_row(row);
            }
            println!("    └{}┘", sep);
            println!("    travel % and fleet t/hr use Bi/walk (most realistic scenario)");
        }
    }

    // -- Scenario (always parametric from embedded config) --
    print_parametric_scenario_info(map, racking, vtc, &scenario_config, &config, verbose);
}

fn print_parametric_scenario_info(
    map: WarehouseMap,
    racking: Racking,
    vmax: u32,
    scenario_config: &ScenarioConfig,
    config: &SimConfig,
    verbose: bool,
) {
    let data = dessim::runner::resolve_scenario_with_racking(map, racking, scenario_config, config)
        .unwrap_or_else(|e| panic!("failed to generate scenario: {}", e));

    let label = match scenario_config {
        ScenarioConfig::Generated(_) => "generated",
        ScenarioConfig::Inline(_) => "inline",
        ScenarioConfig::Csv(_) => "csv",
        ScenarioConfig::BulkSource(_) => "bulk_source",
        ScenarioConfig::SiteExtract(_) => "site_extract",
    };
    print_scenario_data(label, &data, vmax, verbose);
}

fn print_scenario_data(scenario_name: &str, data: &ScenarioData, vmax: u32, verbose: bool) {
    // -- Summary stats --
    let num_skus = data.skus.len();
    let num_popular = data.skus.iter().filter(|s| s.is_popular).count();
    let num_orders = data.orders.len();
    // Inventory: total units and per-slot quantities.
    let total_units: u32 = data.inventory.values().sum();
    let num_slots = data.inventory.len();
    let (slot_min, slot_max, slot_avg) = if num_slots > 0 {
        let min = *data.inventory.values().min().unwrap();
        let max = *data.inventory.values().max().unwrap();
        let avg = total_units as f64 / num_slots as f64;
        (min, max, avg)
    } else {
        (0, 0, 0.0)
    };

    // SKU spread: how many slots each SKU occupies.
    let mut slots_per_sku: BTreeMap<SkuId, u32> = BTreeMap::new();
    for &(sku, _) in data.inventory.keys() {
        *slots_per_sku.entry(sku).or_default() += 1;
    }
    let (spread_min, spread_max, spread_avg) = if !slots_per_sku.is_empty() {
        let min = *slots_per_sku.values().min().unwrap();
        let max = *slots_per_sku.values().max().unwrap();
        let avg = slots_per_sku.values().sum::<u32>() as f64 / slots_per_sku.len() as f64;
        (min, max, avg)
    } else {
        (0, 0, 0.0)
    };

    // Order stats.
    let total_picks: u64 = data.orders.iter().flat_map(|o| &o.lines).map(|l| l.quantity as u64).sum();
    let picks_per_order: Vec<u32> = data.orders.iter().map(|o| o.lines.iter().map(|l| l.quantity).sum()).collect();
    let lines_per_order: Vec<usize> = data.orders.iter().map(|o| o.lines.len()).collect();

    let (ppo_min, ppo_max, ppo_avg) = min_max_avg(&picks_per_order, |v| v as f64);
    let (lpo_min, lpo_max, lpo_avg) = min_max_avg(&lines_per_order, |v| v as f64);

    // Totes.
    let total_totes = data.tote_placements.len();
    let totes_with_inv = data.tote_placements.iter().filter(|t| t.has_tasks).count();

    // Tote multi-occupancy: SKUs and units per tote.
    let mut tote_skus: BTreeMap<StorageSpaceId, BTreeSet<SkuId>> = BTreeMap::new();
    let mut tote_units: BTreeMap<StorageSpaceId, u32> = BTreeMap::new();
    for (&(sku, slot), &qty) in &data.inventory {
        tote_skus.entry(slot).or_default().insert(sku);
        *tote_units.entry(slot).or_default() += qty;
    }
    let occupied_totes = tote_skus.len();
    let sku_counts: Vec<usize> = tote_skus.values().map(|s| s.len()).collect();
    let (sku_occ_min, sku_occ_max, sku_occ_avg) = min_max_avg(&sku_counts, |v| v as f64);
    let unit_counts: Vec<u32> = tote_units.values().copied().collect();
    let (unit_occ_min, unit_occ_max, unit_occ_avg) = min_max_avg(&unit_counts, |v| v as f64);

    // Storage utilization.
    let total_storage_capacity = data.rack_cells.len() as u32 * vmax;

    println!();
    println!("Scenario:      {}", scenario_name);
    println!("  SKUs:          {} ({} popular)", num_skus, num_popular);
    println!("  Inventory:     {} units across {} storage slots", total_units, num_slots);
    println!("  Units/slot:    min={}  max={}  avg={:.1}", slot_min, slot_max, slot_avg);
    println!("  SKU spread:    min={} max={} avg={:.1} slots/SKU", spread_min, spread_max, spread_avg);
    println!("  Orders:        {}", num_orders);
    println!("  Total picks:   {}", total_picks);
    println!("  Picks/order:   min={}  max={}  avg={:.1}", ppo_min, ppo_max, ppo_avg);
    println!("  Lines/order:   min={}  max={}  avg={:.1}", lpo_min, lpo_max, lpo_avg);
    println!("  Totes:         {} ({} with inventory)", total_totes, totes_with_inv);

    println!();
    println!("Tote occupancy ({} occupied totes):", occupied_totes);
    println!("  SKUs/tote:     min={}  max={}  avg={:.2}", sku_occ_min, sku_occ_max, sku_occ_avg);
    println!("  Units/tote:    min={}  max={}  avg={:.2}", unit_occ_min, unit_occ_max, unit_occ_avg);

    println!();
    println!("Storage utilization:");
    let pct = if total_storage_capacity > 0 { num_slots as f64 / total_storage_capacity as f64 * 100.0 } else { 0.0 };
    println!("  Slots filled:  {} / {} ({:.1}%)", num_slots, total_storage_capacity, pct);

    // -- Verbose output --
    if verbose {
        // SKU Catalog
        println!();
        println!("SKU Catalog:");
        for sku in &data.skus {
            let pop = if sku.is_popular { "yes" } else { "no" };
            println!("  SKU{:03}  inventory={}  popular={}", sku.id.0, sku.inventory_units, pop);
        }

        // Inventory Allocations
        println!();
        println!("Inventory Allocations:");
        for (&(sku, slot), &qty) in &data.inventory {
            let rack_idx = slot.0 / vmax;
            let z_index = slot.0 % vmax;
            let cell =
                data.rack_cells.get(&RackId(rack_idx)).map(|c| format!("{}", c.0)).unwrap_or_else(|| "?".to_string());
            println!(
                "  Slot {:<5} (rack {}, z={}, cell {})  SKU{:03} qty={}",
                slot.0, rack_idx, z_index, cell, sku.0, qty
            );
        }

        // Orders
        println!();
        println!("Orders:");
        for order in &data.orders {
            let num_lines = order.lines.len();
            let num_picks: u32 = order.lines.iter().map(|l| l.quantity).sum();
            let items: Vec<String> = order.lines.iter().map(|l| format!("SKU{:03}:{}", l.sku.0, l.quantity)).collect();
            println!("  ORD{:04}  {} lines, {} picks: {}", order.id.0, num_lines, num_picks, items.join(" "));
        }
    }
}

/// (min, max, mean) of `values`. Empty input yields `(0, 0, 0.0)` — bulk-source
/// scenarios carry no pre-formed orders, so callers can pass empty slices freely.
fn min_max_avg<T: Copy + PartialOrd + Default>(values: &[T], to_f64: fn(T) -> f64) -> (T, T, f64) {
    if values.is_empty() {
        return (T::default(), T::default(), 0.0);
    }
    let min = *values.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    let max = *values.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
    let avg = values.iter().map(|&v| to_f64(v)).sum::<f64>() / values.len() as f64;
    (min, max, avg)
}
