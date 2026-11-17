//! Walmart's own outbound/on-hand extracts, resolved into SKUs and orders.
//!
//! Distinct from [`resolve_csv`], which reads a normalized four-column shape.
//! These are the site's files as delivered: 20 outbound columns, three of them
//! trait-bearing, and an on-hand snapshot of the whole DC.
//!
//! [`resolve_csv`]: super::resolve::resolve_csv

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use csv::Reader;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Deserialize;

use crate::model::{ExitPath, OrderContainer, OrderId, OrderLine, OrderSpec, SkuId};
use crate::scenario::{SiteExtractScenario, StockPolicy, UnknownContainerPolicy};
use crate::simtime::SimTime;

use super::types::SkuInfo;

const LB_TO_KG: f32 = 0.453_592_37;

/// Bands `onhand.csv` uses, coarsest-moving first. `outbound.csv`'s
/// Slow/Medium/Fast is a lossy coarsening of these, so the on-hand column is
/// the one read.
const POPULAR_BANDS: [&str; 2] = ["Superhead", "Head"];

#[derive(Deserialize)]
struct OnhandRow {
    item_id: String,
    #[serde(rename = "Velocity_Band")]
    velocity_band: String,
    weight_lbs: f32,
    on_hand_qty: f64,
}

#[derive(Deserialize)]
struct OutboundRow {
    order_id: String,
    sku_id: String,
    order_qty: u32,
    weight_lbs: f32,
    destination_type: String,
    shipping_protection: String,
    order_drop_time: String,
    cutoff_time: String,
}

/// `YYYY-MM-DD HH:MM:SS` to milliseconds on an arbitrary but monotonic epoch.
/// The absolute value is meaningless; only differences are used, and the run
/// rebases them so the first order arrives at zero.
fn parse_drop_time(raw: &str) -> Option<u64> {
    let (date, time) = raw.split_once(' ')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    let mut t = time.split(':');
    let (h, min, sec): (u64, u64, u64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || min > 59 || sec > 60 {
        return None;
    }
    let days = days_from_civil(y, m, day);
    let secs = days.checked_mul(86_400)?.checked_add((h * 3600 + min * 60 + sec) as i64)?;
    u64::try_from(secs).ok()?.checked_mul(1000)
}

/// `HH:MM` to seconds since midnight. The site states a cutoff as a time of
/// day with no date, so this is only half a timestamp.
fn parse_cutoff(raw: &str) -> Option<u64> {
    let (h, m) = raw.trim().split_once(':')?;
    let (h, m): (u64, u64) = (h.parse().ok()?, m.parse().ok()?);
    (h <= 23 && m <= 59).then_some(h * 3600 + m * 60)
}

/// The first time a cutoff comes round at or after the order dropped.
///
/// INFERENCE, and the weakest step in this reader. A cutoff carries no date,
/// so an order dropped after today's cutoff is read as making tomorrow's. That
/// gives a sane 0..24h spread over the real extract -- p10 3.4h, median 12.9h,
/// p90 21.0h -- but an order dropped just after its cutoff could as easily be
/// same-day late as next-day early, and nothing in the file says which. Worth
/// a question to the site before this drives capacity work.
fn next_cutoff_after(drop_ms: u64, cutoff_secs: u64) -> u64 {
    const DAY_MS: u64 = 86_400_000;
    let at = drop_ms / DAY_MS * DAY_MS + cutoff_secs * 1000;
    if at >= drop_ms { at } else { at + DAY_MS }
}

/// Days since 1970-01-01, proleptic Gregorian (Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// One order's rows, accumulated before the lines are sorted.
struct Draft {
    lines: Vec<(SkuId, u32, f32)>,
    shipping: String,
    destination: String,
    drop_ms: u64,
    /// When this order is due out of the building: the first time its stated
    /// cutoff comes round at or after it dropped.
    due_ms: u64,
}

pub fn resolve(cfg: &SiteExtractScenario) -> Result<(Vec<SkuInfo>, Vec<OrderSpec>), String> {
    let (mut skus, key_to_id) = load_onhand(Path::new(&cfg.onhand_csv))?;
    if let Some(path) = &cfg.stock_top_up_csv {
        let reader = Reader::from_path(path).map_err(|e| format!("failed to open {path}: {e}"))?;
        let (units, count) = apply_stock_top_up(reader, &mut skus, &key_to_id).map_err(|e| format!("{path}: {e}"))?;
        eprintln!("Inventory preload top-up: {units} units across {count} SKUs from {path}");
    }
    let orders = load_outbound(Path::new(&cfg.outbound_csv), &key_to_id, cfg.unknown_container)?;
    if cfg.stock == StockPolicy::CoverDemand {
        cover_demand(&mut skus, &orders);
    }
    Ok((skus, orders))
}

fn apply_stock_top_up(
    mut reader: Reader<impl Read>,
    skus: &mut [SkuInfo],
    key_to_id: &FxHashMap<String, SkuId>,
) -> Result<(u64, usize), String> {
    #[derive(Deserialize)]
    struct TopUpRow {
        item_id: String,
        additional_units: u32,
    }
    let mut seen = FxHashSet::default();
    let mut additions = Vec::new();
    let mut units = 0_u64;
    for (i, result) in reader.deserialize().enumerate() {
        let row: TopUpRow = result.map_err(|e| format!("row {}: {e}", i + 2))?;
        let id = *key_to_id.get(&row.item_id).ok_or_else(|| format!("unknown item_id '{}'", row.item_id))?;
        if !seen.insert(id) || row.additional_units == 0 {
            return Err(format!("item_id '{}': duplicate or zero-unit addition", row.item_id));
        }
        let quantity = skus[id.0 as usize]
            .inventory_units
            .checked_add(row.additional_units)
            .ok_or_else(|| format!("item_id '{}': stock quantity overflow", row.item_id))?;
        additions.push((id, quantity));
        units += u64::from(row.additional_units);
    }
    for (id, quantity) in additions {
        skus[id.0 as usize].inventory_units = quantity;
    }
    Ok((units, seen.len()))
}

/// Stock each SKU the orders touch to the lesser of its extracted quantity and
/// what the orders demand; everything else to nothing.
/// This is an explicit reduced-stock experiment. Full warehouse preload uses
/// `AsExtracted`, independently of any order sample.
fn cover_demand(skus: &mut [SkuInfo], orders: &[OrderSpec]) {
    let mut demand: FxHashMap<SkuId, u32> = FxHashMap::default();
    for order in orders {
        for line in &order.lines {
            *demand.entry(line.sku).or_default() += line.quantity;
        }
    }
    for sku in skus.iter_mut() {
        sku.inventory_units = sku.inventory_units.min(demand.get(&sku.id).copied().unwrap_or(0));
    }
}

fn load_onhand(path: &Path) -> Result<(Vec<SkuInfo>, FxHashMap<String, SkuId>), String> {
    let mut reader = Reader::from_path(path).map_err(|e| format!("failed to open {}: {}", path.display(), e))?;
    let mut skus = Vec::new();
    let mut key_to_id = FxHashMap::default();
    for (i, result) in reader.deserialize().enumerate() {
        let row: OnhandRow = result.map_err(|e| format!("{} row {}: {}", path.display(), i + 1, e))?;
        let id = SkuId(i as u32);
        if key_to_id.insert(row.item_id.clone(), id).is_some() {
            return Err(format!("{} row {}: duplicate item_id '{}'", path.display(), i + 1, row.item_id));
        }
        skus.push(SkuInfo {
            id,
            is_popular: POPULAR_BANDS.contains(&row.velocity_band.as_str()),
            inventory_units: row.on_hand_qty.max(0.0).round() as u32,
            weight_kg: row.weight_lbs * LB_TO_KG,
        });
    }
    Ok((skus, key_to_id))
}

fn load_outbound(
    path: &Path,
    key_to_id: &FxHashMap<String, SkuId>,
    unknown: UnknownContainerPolicy,
) -> Result<Vec<OrderSpec>, String> {
    let mut reader = Reader::from_path(path).map_err(|e| format!("failed to open {}: {}", path.display(), e))?;

    // ordered: order ids are the run's order sequence, so they must not depend
    // on hash iteration.
    let mut drafts: BTreeMap<String, Draft> = BTreeMap::new();
    for (i, result) in reader.deserialize().enumerate() {
        let row: OutboundRow = result.map_err(|e| format!("{} row {}: {}", path.display(), i + 1, e))?;
        let sku = *key_to_id.get(&row.sku_id).ok_or_else(|| {
            format!("{} row {}: sku_id '{}' is not in the on-hand file", path.display(), i + 1, row.sku_id)
        })?;
        let cutoff_secs = parse_cutoff(&row.cutoff_time).ok_or_else(|| {
            format!("{} row {}: cutoff_time '{}' is not `HH:MM`", path.display(), i + 2, row.cutoff_time)
        })?;
        let drop_ms = parse_drop_time(&row.order_drop_time).ok_or_else(|| {
            format!(
                "{} row {}: order_drop_time '{}' is not `YYYY-MM-DD HH:MM:SS`",
                path.display(),
                i + 1,
                row.order_drop_time
            )
        })?;
        let line = (sku, row.order_qty, row.weight_lbs * LB_TO_KG);
        match drafts.get_mut(&row.order_id) {
            Some(draft) => {
                if draft.shipping != row.shipping_protection || draft.destination != row.destination_type {
                    return Err(format!(
                        "{} row {}: order '{}' states two different shipping/destination pairs",
                        path.display(),
                        i + 1,
                        row.order_id
                    ));
                }
                draft.lines.push(line);
            }
            None => {
                drafts.insert(
                    row.order_id,
                    Draft {
                        lines: vec![line],
                        shipping: row.shipping_protection,
                        destination: row.destination_type,
                        drop_ms,
                        due_ms: next_cutoff_after(drop_ms, cutoff_secs),
                    },
                );
            }
        }
    }

    // Rebase so the earliest order arrives at zero; the files' absolute dates
    // are not the run's clock.
    let epoch = drafts.values().map(|d| d.drop_ms).min().unwrap_or(0);

    let mut orders = Vec::with_capacity(drafts.len());
    for (i, (key, mut draft)) in drafts.into_iter().enumerate() {
        let (container, is_container_shared) = container_of(&draft.shipping, unknown, &key)?;
        let exit_path = exit_path_of(&draft.destination, &key)?;
        draft.lines.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(Ordering::Equal));
        let lines = draft.lines.into_iter().map(|(sku, quantity, _)| OrderLine { sku, quantity }).collect();
        orders.push(OrderSpec {
            is_container_shared,
            container,
            exit_path,
            arrival: Some(SimTime::ms(draft.drop_ms.saturating_sub(epoch))),
            // Preserve the interpreted cutoff as metadata. Its operational
            // meaning is unconfirmed; it does not delay buffer retrieval.
            deadline: Some(SimTime::ms(draft.due_ms.saturating_sub(epoch))),
            source_label: Some(draft.shipping),
            ..OrderSpec::unstated(OrderId(i as u32), lines)
        });
    }
    Ok(orders)
}

/// `bag` picks into a tote and is bagged downstream; `box` picks into the box
/// and is sealed (Kalpesh, 2026-08-07). Daniel's September 7 provisional SIOC
/// flow uses a reusable tray; the scenario still states its policy explicitly.
fn container_of(
    shipping: &str,
    unknown: UnknownContainerPolicy,
    order: &str,
) -> Result<(OrderContainer, bool), String> {
    let effective = match (shipping, unknown) {
        ("SIOC", UnknownContainerPolicy::AsCarton) => "box",
        ("SIOC", UnknownContainerPolicy::AsTote) => "bag",
        ("SIOC", UnknownContainerPolicy::AsTray) => return Ok((OrderContainer::Tray, true)),
        ("SIOC", UnknownContainerPolicy::Refuse) => {
            return Err(format!(
                "order '{order}' is SIOC and the scenario's unknown_container policy is `refuse`. SIOC's \
                 meaning is not established; set `unknown_container` to `as_tray`, `as_tote` or `as_carton` to state an \
                 assumption for the run"
            ));
        }
        (other, _) => other,
    };
    match effective {
        "bag" => Ok((OrderContainer::Tote, true)),
        "box" => Ok((OrderContainer::Carton, false)),
        other => Err(format!("order '{order}': unknown shipping_protection '{other}'")),
    }
}

/// Daniel's September 7 provisional routing: DS buffers and 3P goes directly
/// to extraction. Keep SC's Sorter label for provenance; fulfillment currently
/// routes it directly to extraction too, pending the site's clarification.
fn exit_path_of(destination: &str, order: &str) -> Result<ExitPath, String> {
    match destination {
        "DS" => Ok(ExitPath::Buffer),
        "SC" => Ok(ExitPath::Sorter),
        "3P" => Ok(ExitPath::Dock),
        other => Err(format!("order '{order}': unknown destination_type '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(unknown: UnknownContainerPolicy) -> SiteExtractScenario {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/walmart_extract");
        SiteExtractScenario {
            outbound_csv: dir.join("outbound.csv").to_string_lossy().into_owned(),
            onhand_csv: dir.join("onhand.csv").to_string_lossy().into_owned(),
            stock_top_up_csv: None,
            unknown_container: unknown,
            units_per_tote: 1,
            order_totes: 0,
            inventory_distribution: Default::default(),
            stock: StockPolicy::AsExtracted,
        }
    }

    #[test]
    fn onhand_becomes_the_sku_catalog() {
        let (skus, _) = resolve(&cfg(UnknownContainerPolicy::AsCarton)).expect("loads");
        assert_eq!(skus.len(), 4);
        assert_eq!(skus.iter().map(|s| s.inventory_units).sum::<u32>(), 185);
        // Superhead + Head are popular; Torso + Tail are not.
        assert_eq!(skus.iter().filter(|s| s.is_popular).count(), 2);
        assert!((skus[0].weight_kg - 2.0 * LB_TO_KG).abs() < 1e-6);
    }

    #[test]
    fn explicit_top_up_changes_only_named_stock_and_rejects_invalid_additions_atomically() {
        let cfg = cfg(UnknownContainerPolicy::AsCarton);
        let (original, keys) = load_onhand(Path::new(&cfg.onhand_csv)).unwrap();
        let key = keys.iter().find(|(_, id)| id.0 == 0).unwrap().0;
        let mut stock = original.clone();
        let csv = format!("item_id,additional_units\n{key},3\n");
        assert_eq!(apply_stock_top_up(Reader::from_reader(csv.as_bytes()), &mut stock, &keys).unwrap(), (3, 1));
        assert_eq!(stock[0].inventory_units, original[0].inventory_units + 3);
        for i in 1..stock.len() {
            assert_eq!(stock[i].inventory_units, original[i].inventory_units);
        }
        for rows in [
            format!("{key},3\n{key},1"),
            format!("{key},3\nmissing,1"),
            format!("{key},0"),
            format!("{key},4294967295"),
        ] {
            let mut stock = original.clone();
            let csv = format!("item_id,additional_units\n{rows}\n");
            assert!(apply_stock_top_up(Reader::from_reader(csv.as_bytes()), &mut stock, &keys).is_err());
            assert_eq!(
                stock.iter().map(|s| s.inventory_units).collect::<Vec<_>>(),
                original.iter().map(|s| s.inventory_units).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn shipping_and_destination_become_traits() {
        let (_, orders) = resolve(&cfg(UnknownContainerPolicy::AsCarton)).expect("loads");
        assert_eq!(orders.len(), 4);

        let bag = &orders[0];
        assert!(bag.is_container_shared);
        assert_eq!(bag.container, OrderContainer::Tote);
        assert_eq!(bag.exit_path, ExitPath::Buffer);

        let boxed = &orders[1];
        assert!(!boxed.is_container_shared);
        assert_eq!(boxed.container, OrderContainer::Carton);
        assert_eq!(boxed.exit_path, ExitPath::Sorter);

        assert_eq!(orders[2].exit_path, ExitPath::Dock);
    }

    /// The order file's own `order_profile` column is never read; cardinality
    /// comes off the lines the reader built.
    #[test]
    fn sku_and_unit_counts_come_from_the_grouped_lines() {
        let (_, orders) = resolve(&cfg(UnknownContainerPolicy::AsCarton)).expect("loads");
        assert_eq!((orders[0].unique_sku_count(), orders[0].unit_count()), (1, 1));

        let multi = &orders[3];
        assert_eq!((multi.unique_sku_count(), multi.unit_count()), (2, 3));
        assert!(multi.is_multi_sku() && multi.is_multi_unit());
        // Heaviest line first.
        assert_eq!(multi.lines[0].sku, SkuId(2));
    }

    #[test]
    fn the_source_label_survives() {
        let (_, orders) = resolve(&cfg(UnknownContainerPolicy::AsCarton)).expect("loads");
        let labels: Vec<_> = orders.iter().map(|o| o.source_label.as_deref()).collect(); // source-label-ok: test
        assert_eq!(labels, vec![Some("bag"), Some("box"), Some("SIOC"), Some("box")]);
    }

    #[test]
    fn sioc_follows_the_stated_policy() {
        let (_, as_carton) = resolve(&cfg(UnknownContainerPolicy::AsCarton)).expect("loads");
        assert_eq!(as_carton[2].container, OrderContainer::Carton);

        let (_, as_tote) = resolve(&cfg(UnknownContainerPolicy::AsTote)).expect("loads");
        assert_eq!(as_tote[2].container, OrderContainer::Tote);

        let (_, as_tray) = resolve(&cfg(UnknownContainerPolicy::AsTray)).expect("loads");
        assert_eq!(as_tray[2].container, OrderContainer::Tray);
        assert!(as_tray[2].is_container_shared);
    }

    #[test]
    fn sioc_refuses_when_the_policy_says_so() {
        let err = resolve(&cfg(UnknownContainerPolicy::Refuse)).expect_err("refuses");
        assert!(err.contains("SIOC"), "{err}");
    }

    #[test]
    fn an_sku_outside_the_onhand_file_is_an_error() {
        let mut c = cfg(UnknownContainerPolicy::AsCarton);
        c.onhand_csv = c.onhand_csv.replace("onhand.csv", "onhand_short.csv");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/walmart_extract");
        std::fs::write(
            dir.join("onhand_short.csv"),
            "item_id,channel,Velocity_Band,length_in,width_in,height_in,weight_lbs,item_cube,on_hand_qty\n\
             9000000001,1P,Head,1,1,1,1,1,5\n",
        )
        .expect("writes");
        let err = resolve(&c).expect_err("refuses");
        std::fs::remove_file(dir.join("onhand_short.csv")).ok();
        assert!(err.contains("not in the on-hand file"), "{err}");
    }

    #[test]
    fn drop_times_become_arrivals_rebased_to_zero() {
        let (_, orders) = resolve(&cfg(UnknownContainerPolicy::AsCarton)).expect("loads");
        // Fixture drops are 09:00, 10:00, 11:00, 12:00 on one day.
        let hour = 3_600_000;
        let got: Vec<u64> = orders.iter().map(|o| o.arrival.expect("stated").as_ms()).collect();
        assert_eq!(got, vec![0, hour, 2 * hour, 3 * hour]);
    }

    /// A cutoff is a time of day, so its date comes from the drop it follows.
    #[test]
    fn a_cutoff_becomes_the_next_time_that_hour_comes_round() {
        assert_eq!(parse_cutoff("13:38"), Some(13 * 3600 + 38 * 60));
        assert_eq!(parse_cutoff("04:28"), Some(4 * 3600 + 28 * 60));
        assert_eq!(parse_cutoff("24:00"), None);
        assert_eq!(parse_cutoff("13"), None);

        const DAY: u64 = 86_400_000;
        let noon = 12 * 3_600_000;
        // Cutoff still ahead today: same day.
        assert_eq!(next_cutoff_after(DAY + noon, 13 * 3600), DAY + 13 * 3_600_000);
        // Cutoff already gone: tomorrow's.
        assert_eq!(next_cutoff_after(DAY + noon, 4 * 3600), 2 * DAY + 4 * 3_600_000);
        // Exactly on it counts as making it.
        assert_eq!(next_cutoff_after(DAY + noon, 12 * 3600), DAY + noon);
    }

    /// A drop time is a full date and time; anything short of one, or out of
    /// range, is refused rather than read as some other instant.
    #[test]
    fn a_malformed_drop_time_is_an_error() {
        assert_eq!(parse_drop_time("2026-08-13 09:00:00"), Some(1_786_611_600_000));
        assert!(parse_drop_time("13/08/2026 09:00:00").is_none());
        assert!(parse_drop_time("2026-08-13").is_none());
        assert!(parse_drop_time("2026-13-13 09:00:00").is_none());
        assert!(parse_drop_time("2026-08-13 25:00:00").is_none());
    }

    /// Cross-check the civil-date arithmetic against a known epoch day.
    #[test]
    fn days_from_civil_matches_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2026, 8, 13), 20_678);
        // A day apart across a month boundary.
        assert_eq!(days_from_civil(2026, 9, 1) - days_from_civil(2026, 8, 31), 1);
    }

    /// The delivered extracts are customer data and live outside the repo, so
    /// this is opt-in:
    ///
    /// ```text
    /// WALMART_EXTRACTS=<dir> cargo test --release -p dessim --lib -- \
    ///     real_extracts --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs WALMART_EXTRACTS"]
    fn real_extracts_reproduce_the_pinned_counts() {
        let Ok(dir) = std::env::var("WALMART_EXTRACTS") else { return };
        let cfg = SiteExtractScenario {
            outbound_csv: format!("{dir}/outbound.csv"),
            onhand_csv: format!("{dir}/onhand.csv"),
            stock_top_up_csv: None,
            unknown_container: UnknownContainerPolicy::AsCarton,
            units_per_tote: 1,
            order_totes: 0,
            inventory_distribution: Default::default(),
            stock: StockPolicy::AsExtracted,
        };
        let (skus, orders) = resolve(&cfg).expect("loads");

        assert_eq!(skus.len(), 174_997);
        assert_eq!(skus.iter().map(|s| s.inventory_units as u64).sum::<u64>(), 1_397_589);
        assert_eq!(skus.iter().filter(|s| s.is_popular).count(), 12_641);

        assert_eq!(orders.len(), 22_160);
        assert_eq!(orders.iter().map(|o| o.unique_sku_count()).sum::<usize>(), 24_789);
        assert_eq!(orders.iter().filter(|o| o.container == OrderContainer::Tote).count(), 10_194);
        assert_eq!(orders.iter().filter(|o| o.exit_path == ExitPath::Buffer).count(), 14_626);
        assert_eq!(orders.iter().filter(|o| o.exit_path == ExitPath::Sorter).count(), 3_545);
        assert_eq!(orders.iter().filter(|o| o.exit_path == ExitPath::Dock).count(), 3_989);

        // 1,334, not the 1,337 the order_profile column claims: three MLMU
        // orders carry one line. Derived cardinality is right where the label
        // is wrong.
        assert_eq!(orders.iter().filter(|o| o.is_multi_sku()).count(), 1_334);
        assert_eq!(orders.iter().filter(|o| o.is_multi_unit()).count(), 1_647);

        // A real 47-hour arrival curve, rebased so the first order is zero.
        let releases: Vec<u64> = orders.iter().map(|o| o.arrival.expect("stated").as_ms()).collect();
        assert_eq!(releases.iter().copied().min(), Some(0));
        let span_h = releases.iter().copied().max().unwrap() / 3_600_000;
        assert_eq!(span_h, 47);
        let mut per_hour = [0usize; 48];
        for r in &releases {
            per_hour[(r / 3_600_000) as usize] += 1;
        }
        eprintln!("orders per hour: {per_hour:?}");
        assert!(per_hour.iter().max().unwrap() > &1_000, "a real peak");
        assert!(per_hour[0] < 100, "the curve starts quiet");
    }
}
