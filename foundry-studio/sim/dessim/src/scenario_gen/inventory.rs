//! Within `scenario_gen`, `inventory` is WHAT IS IN THE BUILDING AT T=0 and
//! WHERE: which slot each tote starts in, which pool each tote belongs to for
//! its whole life, and how many units of each SKU sit in which slot.
//!
//! Two placements, one per storage system: relay totes on rack columns, and
//! RTP racks on storage cells. Both take the map's racking as given and only
//! decide what occupies it, and the map decides that too: each slot's
//! [`RackUse`] says which pool its tote belongs to, or that it holds no tote
//! at all. Nothing here counts totes into pools -- a slot's designation is
//! WHERE it is.

use std::collections::{BTreeMap, BTreeSet};

use desgraph::{CellId, StationId};
use desmap::Station;
use rustc_hash::FxHashSet;

use crate::model::{RackId, ToteDesignation, ToteId, ToteLocation};
use crate::model::{SkuId, StorageSpaceId};
use crate::scenario::InventoryDistribution;

use super::topology::RackColumn;
use super::types::{RackPlacement, SkuInfo, TotePlacement};

/// Relay system: create totes on storage columns, distribute inventory.
///
/// Each storage column (`racks[i]`) is one rack (`RackId(i)`), and each rack
/// has `vertical_tote_capacity` tote slots indexed by z_index (0..vMax).
/// `StorageSpaceId(x) == ToteId(x)` via `x = i * vMax + z_index`. By default
/// there is one column per storage cell; zones with `slots_per_row` set may
/// anchor several columns to the same cell.
///
/// Returns (inventory_map, tote_placements, rack_cells).
pub(super) fn generate_relay_inventory(
    skus: &[SkuInfo],
    racks: &[RackColumn],
    vertical_tote_capacity: u32,
    units_per_tote: u32,
    distribution: InventoryDistribution,
    order_totes: u32,
    preload_fraction: f64,
) -> Result<(BTreeMap<(SkuId, StorageSpaceId), u32>, Vec<TotePlacement>, BTreeMap<RackId, CellId>), String> {
    let vmax = vertical_tote_capacity.max(1);
    let total_slots = racks.len() * vmax as usize;

    let mut inventory: BTreeMap<(SkuId, StorageSpaceId), u32> = BTreeMap::new();
    let mut has_tasks_set = BTreeSet::new();

    let mut rack_cells: BTreeMap<RackId, CellId> = BTreeMap::new();
    for (i, rack) in racks.iter().enumerate() {
        rack_cells.insert(RackId(i as u32), rack.cell);
    }

    // Build a shuffled slot index so each SKU's inventory is scattered
    // across the warehouse rather than packed into contiguous cells.
    // Uses Fisher-Yates with a fixed seed for determinism.
    //
    // Only inventory racking takes stock. The order buffer holds picked orders
    // waiting to go out and the order-tote row holds empty order totes, and
    // neither is anywhere to put a SKU. Slot ids stay flat over ALL racks so
    // `RackId` and `StorageSpaceId` keep meaning the same thing; only the
    // PLACEMENT order skips them.
    let mut slot_order = scatter((0..total_slots).filter(|slot| racks[slot / vmax as usize].uses.takes_stock()));

    if distribution == InventoryDistribution::Even {
        slot_order.truncate((slot_order.len() as f64 * preload_fraction).ceil() as usize);
        inventory = distribute_evenly(skus, &slot_order, units_per_tote)?;
        has_tasks_set.extend(inventory.keys().map(|(_, space)| space.0 as usize));
    } else {
        let mut cursor = 0usize;
        for sku in skus {
            let mut remaining = sku.inventory_units;
            while remaining > 0 && cursor < slot_order.len() {
                let slot_idx = slot_order[cursor];
                let space = StorageSpaceId(slot_idx as u32);
                let place = remaining.min(units_per_tote.max(1));
                inventory.insert((sku.id, space), place);
                has_tasks_set.insert(slot_idx);
                remaining -= place;
                cursor += 1;
            }
        }
    }

    // A tote per slot, designated by WHERE ITS SLOT IS.
    //
    // Inventory racking gets a stock tote; the order-tote row gets an empty
    // ORDER tote; the order buffer gets NOTHING, because a buffer slot is
    // capacity for a closed order the run has not produced yet and a tote
    // parked in one at t=0 spends that capacity. Seeding every slot left
    // DFW6's 3,450 buffer slots fully occupied by empty inventory totes,
    // which both made a buffer deposit impossible and turned the buffer into
    // putaway's nearest empty-tote depot.
    //
    // `order_totes` is a CAP, not the designation: a document that declares an
    // order pool smaller than its order racking parks only that many totes in
    // the row and leaves the rest of it as landing space for order totes
    // coming back from extraction. Zero means the map's row decides alone.
    // DFW6 declares a pool the size of its row, so there the cap binds
    // nothing; a site that wants fewer totes than positions says so and gets
    // them.
    //
    // Slot ids stay flat over ALL racks, so `RackId`/`StorageSpaceId`/`ToteId`
    // keep meaning the same thing; the buffer's flat indices simply name no
    // tote.
    //
    // A cap is spent over a SCATTERED reading of the row, not its first slots:
    // taken in rack order, a pool smaller than the row lands entirely in the
    // first aisle's row bay and every close-out in the building queues on that
    // one aisle.
    let depot: FxHashSet<usize> = {
        let all = scatter(
            (0..total_slots).filter(|slot| racks[slot / vmax as usize].uses.pool() == Some(ToteDesignation::Order)),
        );
        let take = if order_totes == 0 { all.len() } else { (order_totes as usize).min(all.len()) };
        all.into_iter().take(take).collect()
    };

    let mut tote_placements = Vec::with_capacity(total_slots);
    for i in 0..racks.len() {
        let Some(designation) = racks[i].uses.pool() else { continue };
        let rack_id = RackId(i as u32);
        for z in 0..vmax {
            let flat_idx = i * vmax as usize + z as usize;
            if designation == ToteDesignation::Order && !depot.contains(&flat_idx) {
                continue;
            }
            tote_placements.push(TotePlacement {
                tote_id: ToteId(flat_idx as u32),
                location: ToteLocation::InRack { rack_id, z_index: z as u16 },
                has_tasks: has_tasks_set.contains(&flat_idx),
                designation,
            });
        }
    }

    // Diagnostic: per-rack inventory distribution + zone coverage.
    {
        let num_racks = racks.len();
        let mut racks_with_inv = BTreeSet::new();
        for &slot in &has_tasks_set {
            racks_with_inv.insert(slot / vmax as usize);
        }
        eprintln!(
            "[inventory] {} SKUs, {} units in {} stocked totes / {} slots across {}/{} racks (vtc={}, units_per_tote={}, distribution={distribution:?})",
            skus.len(),
            inventory.values().map(|&qty| u64::from(qty)).sum::<u64>(),
            has_tasks_set.len(),
            total_slots,
            racks_with_inv.len(),
            num_racks,
            vmax,
            units_per_tote,
        );
    }

    Ok((inventory, tote_placements, rack_cells))
}

/// Fill each scattered inventory slot to its share of the total. Keeping SKU
/// quantities together until a tote fills avoids needlessly fragmenting a SKU
/// into one-unit pieces; adjacent SKUs can share the remaining space.
fn distribute_evenly(
    skus: &[SkuInfo],
    slots: &[usize],
    units_per_tote: u32,
) -> Result<BTreeMap<(SkuId, StorageSpaceId), u32>, String> {
    // ordered: downstream inventory registration follows (SKU, storage slot).
    let mut inventory = BTreeMap::new();
    let total = skus.iter().map(|sku| u64::from(sku.inventory_units)).sum::<u64>();
    if total == 0 {
        return Ok(inventory);
    }
    let count = slots.len() as u64;
    if count == 0 {
        return Err(format!("cannot preload {total} units: no inventory tote positions"));
    }
    let largest = total.div_ceil(count);
    if largest > u64::from(units_per_tote) {
        return Err(format!(
            "cannot evenly preload {total} units into {count} inventory totes: \
             requires {largest} units per tote, above units_per_tote={units_per_tote}"
        ));
    }
    let share = |index: usize| (total / count + u64::from((index as u64) < total % count)) as u32;
    let mut cursor = 0;
    let mut available = share(cursor);
    for sku in skus {
        let mut remaining = sku.inventory_units;
        while remaining > 0 {
            if available == 0 {
                cursor += 1;
                available = share(cursor);
            }
            let quantity = remaining.min(available);
            let slot = u32::try_from(slots[cursor]).map_err(|_| "inventory slot id exceeds u32".to_owned())?;
            *inventory.entry((sku.id, StorageSpaceId(slot))).or_default() += quantity;
            available -= quantity;
            remaining -= quantity;
        }
    }
    Ok(inventory)
}

/// RTP system: create racks on storage cells, distribute inventory.
///
/// Returns (inventory_map, rack_placements).
pub(super) fn generate_rtp_inventory(
    skus: &[SkuInfo],
    storage_cells: &[CellId],
    stations: &[Station],
    units_per_tote: u32,
) -> (BTreeMap<(SkuId, StorageSpaceId), u32>, Vec<RackPlacement>) {
    let mut inventory: BTreeMap<(SkuId, StorageSpaceId), u32> = BTreeMap::new();
    let mut rack_sku_counts: BTreeMap<u32, u32> = BTreeMap::new();

    let mut cell_idx = 0usize;
    for sku in skus {
        let mut remaining = sku.inventory_units;
        while remaining > 0 && !storage_cells.is_empty() {
            let space = StorageSpaceId(cell_idx as u32);
            let place = remaining.min(units_per_tote);
            inventory.insert((sku.id, space), place);
            *rack_sku_counts.entry(cell_idx as u32).or_insert(0) += place;
            remaining -= place;
            cell_idx = (cell_idx + 1) % storage_cells.len();
        }
    }

    let default_station = stations.first().map(|s| s.id).unwrap_or(StationId(0));

    let rack_placements: Vec<RackPlacement> = storage_cells
        .iter()
        .enumerate()
        .map(|(i, &cell_id)| {
            let mut tasks_by_station = BTreeMap::new();
            if let Some(&count) = rack_sku_counts.get(&(i as u32)) {
                tasks_by_station.insert(default_station, count);
            }
            RackPlacement { rack_id: RackId(i as u32), pos: cell_id, tasks_by_station }
        })
        .collect();

    (inventory, rack_placements)
}

/// Fisher-Yates over a fixed seed: the same slots in the same order on every
/// run, spread over the building rather than packed into contiguous steel.
fn scatter(slots: impl Iterator<Item = usize>) -> Vec<usize> {
    let mut order: Vec<usize> = slots.collect();
    let mut rng_state: u64 = 0xDEAD_BEEF_CAFE_1234;
    for i in (1..order.len()).rev() {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 7;
        rng_state ^= rng_state << 17;
        let j = (rng_state as usize) % (i + 1);
        order.swap(i, j);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario_gen::topology::{RackColumn, RackUse};

    fn rack(uses: RackUse) -> RackColumn {
        RackColumn { cell: CellId(0), world_pos: None, floor_m: None, side: None, uses }
    }

    /// An order buffer holds picked orders waiting to go out. Seeding stock
    /// into it fills the very slots the buffer flow will need, and shows up as
    /// a capacity problem later rather than a placement one.
    #[test]
    fn inventory_never_lands_in_an_order_buffer() {
        // Racks 0..3 are storage; 4..7 are buffer.
        let racks: Vec<RackColumn> =
            (0..8).map(|i| rack(if i < 4 { RackUse::Inventory } else { RackUse::OrderBuffer })).collect();
        let skus: Vec<SkuInfo> =
            (0..4).map(|i| SkuInfo { id: SkuId(i), is_popular: false, inventory_units: 1, weight_kg: 0.0 }).collect();

        let (inventory, placements, _) =
            generate_relay_inventory(&skus, &racks, 1, 1, InventoryDistribution::SingleSku, 0, 1.0).unwrap();

        assert_eq!(inventory.len(), 4, "every unit placed");
        for (sku, space) in inventory.keys() {
            assert!((space.0 as usize) < 4, "sku {sku:?} landed in buffer slot {space:?}; storage slots are 0..4");
        }
        // The buffer starts empty of totes: its slots are capacity for closed
        // orders, not somewhere to park stock. Slot ids stay flat over every
        // rack, so the four that exist are the four storage ones.
        assert_eq!(placements.len(), 4, "a tote per storage slot, none in the buffer");
        assert!(
            placements.iter().all(|p| (p.tote_id.0 as usize) < 4),
            "no tote id names a buffer slot: {:?}",
            placements.iter().map(|p| p.tote_id).collect::<Vec<_>>()
        );
    }

    /// The two pools never overlap, and an order tote is never a stock tote
    /// that happened to be empty. Permanent designation is the anti-starvation
    /// ruling: if either side could draw from the other, one of them
    /// eventually cannot work.
    ///
    /// Which tote is which is now a question about STEEL: the order pool is
    /// exactly the totes standing in order-depot racking, so the two pools
    /// cannot overlap by construction rather than by a tail-of-the-shuffle
    /// carve-out that only held while the inventory fill fell short.
    #[test]
    fn order_and_inventory_totes_are_disjoint_pools() {
        // Racks 0..16 are stock; 16..20 are the row of empty order totes.
        let racks: Vec<RackColumn> =
            (0..20).map(|i| rack(if i < 16 { RackUse::Inventory } else { RackUse::OrderDepot })).collect();
        let skus: Vec<SkuInfo> =
            (0..5).map(|i| SkuInfo { id: SkuId(i), is_popular: false, inventory_units: 1, weight_kg: 0.0 }).collect();

        let (inventory, placements, _) =
            generate_relay_inventory(&skus, &racks, 1, 1, InventoryDistribution::SingleSku, 0, 1.0).unwrap();

        let order: Vec<u32> =
            placements.iter().filter(|p| p.designation == ToteDesignation::Order).map(|p| p.tote_id.0).collect();
        assert_eq!(order.len(), 4, "one order tote per slot of the row, and nowhere else");
        assert!(order.iter().all(|&t| t >= 16), "an order tote stands in the row: {order:?}");

        // No order tote holds stock.
        for (_, space) in inventory.keys() {
            assert!(!order.contains(&space.0), "slot {space:?} is both an order tote and seeded with inventory");
        }
        // And no order tote was given work.
        for p in placements.iter().filter(|p| p.designation == ToteDesignation::Order) {
            assert!(!p.has_tasks, "order tote {:?} was seeded with a pick", p.tote_id);
        }
    }

    /// A declared carrier pool smaller than the row is a CAP on how much of
    /// the row starts occupied, and it is spent over a scattered reading of
    /// the row: taken in rack order, every order tote in DFW6 would stand in
    /// one aisle and every close-out in the building would queue on it.
    #[test]
    fn a_declared_pool_caps_the_row_and_scatters_over_it() {
        let racks: Vec<RackColumn> =
            (0..20).map(|i| rack(if i < 4 { RackUse::Inventory } else { RackUse::OrderDepot })).collect();

        let (_, placements, _) =
            generate_relay_inventory(&[], &racks, 1, 1, InventoryDistribution::SingleSku, 5, 1.0).unwrap();

        let order: Vec<u32> =
            placements.iter().filter(|p| p.designation == ToteDesignation::Order).map(|p| p.tote_id.0).collect();
        assert_eq!(order.len(), 5, "the pool is the size the document declared");
        assert!(order.iter().all(|&t| t >= 4), "every order tote stands in the row: {order:?}");
        assert!(
            order.iter().any(|&t| t > 8),
            "the pool is spread over the row, not packed into its first slots: {order:?}"
        );
    }

    /// Stock racking is untouched by any of this: a map that states no order
    /// racking places exactly what it placed before the kind existed.
    #[test]
    fn all_inventory_racks_place_as_before() {
        let racks: Vec<RackColumn> = (0..8).map(|_| rack(RackUse::Inventory)).collect();
        let skus: Vec<SkuInfo> =
            (0..8).map(|i| SkuInfo { id: SkuId(i), is_popular: false, inventory_units: 1, weight_kg: 0.0 }).collect();
        let (inventory, _, _) =
            generate_relay_inventory(&skus, &racks, 1, 1, InventoryDistribution::SingleSku, 0, 1.0).unwrap();
        assert_eq!(inventory.len(), 8);
    }

    /// The fraction confines the even preload to that share of the inventory totes, leaving the others empty for
    /// putaway. Single-SKU totes past the stock start empty anyway, so there the cut would only drop stock.
    #[test]
    fn only_the_even_preload_is_confined_to_the_fraction_of_the_inventory_totes() {
        let racks: Vec<RackColumn> = (0..8).map(|_| rack(RackUse::Inventory)).collect();
        let skus: Vec<SkuInfo> =
            (0..6).map(|i| SkuInfo { id: SkuId(i), is_popular: false, inventory_units: 1, weight_kg: 0.0 }).collect();
        let stocked = |distribution| {
            let (_, placements, _) = generate_relay_inventory(&skus, &racks, 1, 100, distribution, 0, 0.5).unwrap();
            placements.iter().filter(|placement| placement.has_tasks).count()
        };
        assert_eq!(stocked(InventoryDistribution::SingleSku), 6, "a tote for each SKU");
        assert_eq!(stocked(InventoryDistribution::Even), 4, "half the inventory totes");
    }

    #[test]
    fn multiunit_totes_preserve_sku_stock_and_leave_a_partial_last_tote() {
        let racks: Vec<RackColumn> = (0..8).map(|_| rack(RackUse::Inventory)).collect();
        let skus = vec![
            SkuInfo { id: SkuId(0), is_popular: false, inventory_units: 250, weight_kg: 0.0 },
            SkuInfo { id: SkuId(1), is_popular: false, inventory_units: 1, weight_kg: 0.0 },
        ];
        let (inventory, placements, _) =
            generate_relay_inventory(&skus, &racks, 1, 100, InventoryDistribution::SingleSku, 0, 1.0).unwrap();
        let mut first_sku: Vec<_> =
            inventory.iter().filter_map(|(&(sku, _), &qty)| (sku == SkuId(0)).then_some(qty)).collect();
        first_sku.sort_unstable();
        assert_eq!(first_sku, [50, 100, 100]);
        assert_eq!(inventory.values().sum::<u32>(), 251, "capacity changes packing, not the supplied stock");
        assert_eq!(inventory.len(), 4);
        assert_eq!(placements.iter().filter(|placement| placement.has_tasks).count(), 4);
    }

    #[test]
    fn even_preload_conserves_each_sku_and_balances_only_inventory_totes() {
        let racks = vec![
            rack(RackUse::Inventory),
            rack(RackUse::OrderDepot),
            rack(RackUse::Inventory),
            rack(RackUse::OrderBuffer),
        ];
        let skus: Vec<_> = [13, 2, 3, 4, 1, 0]
            .into_iter()
            .enumerate()
            .map(|(id, inventory_units)| SkuInfo {
                id: SkuId(id as u32),
                is_popular: false,
                inventory_units,
                weight_kg: 0.0,
            })
            .collect();
        let (inventory, placements, _) =
            generate_relay_inventory(&skus, &racks, 2, 100, InventoryDistribution::Even, 0, 1.0).unwrap();
        let repeated = generate_relay_inventory(&skus, &racks, 2, 100, InventoryDistribution::Even, 0, 1.0).unwrap();
        assert_eq!(inventory, repeated.0, "preload is deterministic");
        let mut totals = [0_u32; 8];
        let mut sku_counts = [0_u32; 8];
        for (&(_, space), &quantity) in &inventory {
            assert!(racks[space.0 as usize / 2].uses.takes_stock());
            totals[space.0 as usize] += quantity;
            sku_counts[space.0 as usize] += 1;
        }
        for sku in &skus {
            assert_eq!(
                inventory.iter().filter(|((id, _), _)| *id == sku.id).map(|(_, qty)| qty).sum::<u32>(),
                sku.inventory_units
            );
        }
        let mut occupied: Vec<_> = totals.into_iter().filter(|qty| *qty > 0).collect();
        occupied.sort_unstable();
        assert_eq!(occupied, [5, 6, 6, 6]);
        assert!(sku_counts.iter().any(|count| *count > 1), "some totes contain multiple SKUs");
        assert!(inventory.values().any(|qty| *qty > 1), "a SKU can contribute multiple units to a tote");
        assert_eq!(placements.iter().filter(|p| p.has_tasks).count(), 4);
        assert!(placements.iter().filter(|p| p.designation == ToteDesignation::Order).all(|p| !p.has_tasks));
    }

    #[test]
    fn even_preload_handles_sparse_empty_and_unplaceable_stock() {
        let skus = [SkuInfo { id: SkuId(0), is_popular: false, inventory_units: 2, weight_kg: 0.0 }];
        let inventory = distribute_evenly(&skus, &[0, 1, 2, 3], 100).unwrap();
        assert_eq!(inventory.values().copied().collect::<Vec<_>>(), [1, 1]);
        assert!(distribute_evenly(&[], &[], 100).unwrap().is_empty());
        assert!(distribute_evenly(&skus, &[], 100).unwrap_err().contains("no inventory tote positions"));
        assert!(distribute_evenly(&skus, &[0], 1).unwrap_err().contains("requires 2 units per tote"));
        assert!(distribute_evenly(&skus, &[0, 1], 0).is_err());
    }
}
