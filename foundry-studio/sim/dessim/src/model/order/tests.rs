use std::fs;
use std::path::Path;

use crate::model::{OrderId, SimTime, SkuId};

use super::{ExitPath, OrderContainer, OrderLine, OrderService, OrderSpec};

fn order(id: u32, skus: &[(u32, u32)]) -> OrderSpec {
    OrderSpec::unstated(OrderId(id), skus.iter().map(|&(s, q)| OrderLine { sku: SkuId(s), quantity: q }).collect())
}

#[test]
fn pick_units_yields_one_entry_per_unit_in_sequence() {
    let order = order(1, &[(10, 2), (20, 1), (30, 3)]);
    let units: Vec<(usize, SkuId)> = order.pick_units().collect();
    assert_eq!(
        units,
        [(0, SkuId(10)), (0, SkuId(10)), (1, SkuId(20)), (2, SkuId(30)), (2, SkuId(30)), (2, SkuId(30))],
        "a pick chain retrieves one unit at a time; line_index groups them"
    );
}

#[test]
fn a_zero_quantity_line_contributes_no_units() {
    let order = order(1, &[(10, 0), (20, 2)]);
    let units: Vec<(usize, SkuId)> = order.pick_units().collect();
    assert_eq!(units, [(1, SkuId(20)), (1, SkuId(20))]);
}

#[test]
fn cardinality_is_derived_from_the_lines() {
    let slsu = order(1, &[(1, 1)]);
    assert_eq!((slsu.unique_sku_count(), slsu.unit_count()), (1, 1));
    assert!(!slsu.is_multi_sku() && !slsu.is_multi_unit());

    let slmu = order(1, &[(1, 3)]);
    assert_eq!((slmu.unique_sku_count(), slmu.unit_count()), (1, 3));
    assert!(!slmu.is_multi_sku() && slmu.is_multi_unit());

    let mlmu = order(1, &[(1, 2), (2, 1), (3, 4)]);
    assert_eq!((mlmu.unique_sku_count(), mlmu.unit_count()), (3, 7));
    assert!(mlmu.is_multi_sku() && mlmu.is_multi_unit());
}

#[test]
fn a_sku_repeated_across_lines_counts_once() {
    let repeated = order(1, &[(1, 2), (1, 3)]);
    assert_eq!((repeated.unique_sku_count(), repeated.unit_count()), (1, 5));
    assert!(!repeated.is_multi_sku() && repeated.is_multi_unit());
}

#[test]
fn an_empty_order_is_neither_multi_sku_nor_multi_unit() {
    let empty = order(1, &[]);
    assert_eq!((empty.unique_sku_count(), empty.unit_count()), (0, 0));
    assert!(!empty.is_multi_sku() && !empty.is_multi_unit());
}

#[test]
fn the_registry_holds_orders_ascending_by_id() {
    let registry = OrderService::new();
    registry.extend([order(9, &[(10, 2)]), order(2, &[(30, 1)])]);

    let ids: Vec<OrderId> = registry.with(|orders| orders.values().map(|o| o.id).collect());
    assert_eq!(ids, [OrderId(2), OrderId(9)]);
    assert_eq!(registry.len(), 2);
    assert!(!registry.is_empty());
}

#[test]
fn an_empty_registry_reports_itself_empty() {
    let registry = OrderService::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert!(registry.snapshot().is_empty());
    assert_eq!(registry.get(OrderId(1)), None);
}

#[test]
fn an_order_keeps_its_traits_through_the_registry() {
    let registry = OrderService::new();
    let mut stated = order(9, &[(10, 2)]);
    stated.container = OrderContainer::Carton;
    stated.is_container_shared = true;
    stated.exit_path = ExitPath::Buffer;
    stated.arrival = Some(SimTime::seconds(30));
    stated.deadline = Some(SimTime::seconds(300));
    stated.source_label = Some("source-owned classification".into());

    registry.insert(stated.clone());
    assert_eq!(registry.get(OrderId(9)), Some(stated));
}

#[test]
fn a_snapshot_is_detached_from_the_registry() {
    let registry = OrderService::new();
    registry.extend([order(2, &[(30, 1)]), order(9, &[(10, 2)])]);

    let mut detached = registry.snapshot();
    detached[1].container = OrderContainer::Carton;

    assert_eq!(registry.get(OrderId(9)).unwrap().container, OrderContainer::Tote, "caller's copy changed the registry");
}

#[test]
fn source_label_is_never_read_by_the_solver() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("readable source dir") {
            let path = entry.expect("readable dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            // Contains the pattern it greps for.
            if path.ends_with("model/order/tests.rs") {
                continue;
            }
            let text = fs::read_to_string(&path).expect("readable source file");
            for (n, line) in text.lines().enumerate() {
                if line.contains("source-label-ok:") {
                    continue;
                }
                let code = line.split("//").next().unwrap_or("");
                // Producers may write it; any other use is a read.
                let Some(after) = code.split(".source_label").nth(1) else { continue };
                let is_write = {
                    let rest = after.trim_start();
                    rest.starts_with('=') && !rest.starts_with("==")
                };
                if !is_write {
                    offenders.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "source_label is inert: a reader may set it, the solver may not read it. If a flow \
         needs the distinction, give it a field on OrderSpec; if the read is legitimate (a test, \
         a report), waive the line with `// source-label-ok: <reason>`.\n{}",
        offenders.join("\n")
    );
}
