#!/usr/bin/env python3
"""Prepare a finite, source-derived Walmart backlog without inventing stock.

Usage: python3 scripts/prepare_walmart_throughput.py --output-dir /absolute/run/folder

Default: all source orders with their original arrival times. No orders are
discarded. Explicit top-ups from the template are copied separately. The manifest states source shortages;
the simulator's inventory validation remains enabled. For an internal ready
backlog, explicitly pass --orders 4000 --arrivals ready. Only an explicit subset
uses deterministic stock-feasible replacement of whole orders.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import math
from collections import Counter
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
FIXTURES = REPO / "sim/dessim/tests/fixtures"
T0 = "2026-09-07 00:00:00"
U32_MAX = (1 << 32) - 1


def read_csv(path: Path, required: set[str]) -> tuple[list[str], list[dict[str, str]]]:
    with path.open(newline="", encoding="utf-8") as source:
        reader = csv.DictReader(source)
        fields = reader.fieldnames or []
        if len(fields) != len(set(fields)) or not required.issubset(fields):
            raise ValueError(f"{path}: duplicate headers or missing {sorted(required - set(fields))}")
        rows = list(reader)
    if any(None in row or any(value is None for value in row.values()) for row in rows):
        raise ValueError(f"{path}: row width does not match CSV headers")
    return fields, rows


def rounded_on_hand(value: str) -> int:
    """Match Rust f64::max(0).round() as u32 (ties away from zero, saturating)."""
    number = float(value)
    if math.isnan(number) or number <= 0:
        return 0
    if number >= U32_MAX:
        return U32_MAX
    integral = math.floor(number)
    return integral + (number - integral >= 0.5)


def choose_orders(
    rows: list[dict[str, str]], stock: list[dict[str, str]], target: int | None
) -> tuple[set[str], list[dict], list[str], list[str]]:
    supply: dict[str, int] = {}
    for row in stock:
        sku = row["item_id"]
        if sku in supply:
            raise ValueError(f"duplicate on-hand item_id {sku!r}")
        supply[sku] = rounded_on_hand(row["on_hand_qty"])
    demand: dict[str, Counter] = {}
    traits: dict[str, tuple[str, str]] = {}
    for row in rows:
        order = row["order_id"]
        quantity = int(row["order_qty"])
        if not 0 <= quantity <= U32_MAX:
            raise ValueError(f"order {order!r}: quantity is outside u32")
        pair = row["shipping_protection"], row["destination_type"]
        if order in traits and traits[order] != pair:
            raise ValueError(f"order {order!r}: inconsistent packaging/destination")
        traits[order] = pair
        demand.setdefault(order, Counter())[row["sku_id"]] += quantity
    ids = sorted(demand)
    if target is None:
        return set(ids), [], ids, []
    if target <= 0 or target > len(ids):
        raise ValueError(f"requested {target} orders from {len(ids)} distinct source orders")
    primary = [ids[i * len(ids) // target] for i in range(target)]
    selected: set[str] = set()
    rejected: list[dict] = []
    replacements: list[str] = []

    def take(order: str, phase: str) -> bool:
        missing = {
            sku: quantity - supply.get(sku, 0)
            for sku, quantity in sorted(demand[order].items())
            if sku not in supply or quantity > supply[sku]
        }
        if missing:
            rejected.append({"order_id": order, "phase": phase, "short_units_by_sku": missing})
            return False
        for sku, quantity in demand[order].items():
            supply[sku] -= quantity
        selected.add(order)
        return True

    for order in primary:
        take(order, "primary")
    primary_set = set(primary)
    for order in ids:
        if len(selected) == target:
            break
        # Failed primary candidates cannot become feasible as stock decreases.
        if order not in primary_set and take(order, "replacement"):
            replacements.append(order)
    if len(selected) != target:
        raise ValueError(
            f"only {len(selected)} of {target} requested orders are feasible under the "
            "deterministic primary-then-replacement stock allocation; no output written"
        )
    return selected, rejected, primary, replacements


def stock_audit(rows: list[dict[str, str]], stock: list[dict[str, str]]) -> dict:
    """Report finite-source deficits without changing orders or source stock."""
    supply = {row["item_id"]: rounded_on_hand(row["on_hand_qty"]) for row in stock}
    demand: Counter[str] = Counter()
    for row in rows:
        demand[row["sku_id"]] += int(row["order_qty"])
    deficits = {
        sku: {"demand": quantity, "on_hand": supply.get(sku, 0), "short_units": quantity - supply.get(sku, 0)}
        for sku, quantity in sorted(demand.items())
        if quantity > supply.get(sku, 0)
    }
    affected = sorted({row["order_id"] for row in rows if row["sku_id"] in deficits})
    return {
        "short_sku_count": len(deficits),
        "short_unit_count": sum(value["short_units"] for value in deficits.values()),
        "orders_referencing_short_skus": len(affected),
        "order_ids_referencing_short_skus": affected,
        "deficits_by_sku": deficits,
    }


def csv_bytes(fields: list[str], rows: list[dict[str, str]]) -> bytes:
    stream = io.StringIO(newline="")
    writer = csv.DictWriter(stream, fieldnames=fields, lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows)
    return stream.getvalue().encode("utf-8")


def json_bytes(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def prepare(args: argparse.Namespace) -> dict:
    output = args.output_dir.expanduser().resolve()
    if output.is_relative_to(FIXTURES.resolve()):
        raise ValueError("output must be outside the tracked fixture directory")
    paths = {
        "outbound": args.outbound.expanduser().resolve(),
        "onhand": args.onhand.expanduser().resolve(),
        "world": args.world.expanduser().resolve(),
        "template_spec": (FIXTURES / (
            "walmart_dfw6_spec.json" if args.orders is None else "walmart_dfw6_mixed_spec.json"
        )).resolve(),
    }
    out_fields, rows = read_csv(
        paths["outbound"],
        {"order_id", "sku_id", "order_qty", "shipping_protection", "destination_type", "order_drop_time"},
    )
    stock_fields, stock = read_csv(paths["onhand"], {"item_id", "on_hand_qty"})
    selected, rejected, primary, replacements = choose_orders(rows, stock, args.orders)
    selected_rows = [
        dict(row, order_drop_time=T0) if args.arrivals == "ready" else dict(row)
        for row in rows if row["order_id"] in selected
    ]
    used_skus = {row["sku_id"] for row in selected_rows}
    orders = {row["order_id"]: row for row in selected_rows}
    units = sum(int(row["order_qty"]) for row in selected_rows)
    counts = {
        "orders": len(orders),
        "lines": len(selected_rows),
        "units": units,
        "skus": len(used_skus),
        "packaging": dict(sorted(Counter(row["shipping_protection"] for row in orders.values()).items())),
        "routing": dict(sorted(Counter(row["destination_type"] for row in orders.values()).items())),
    }
    spec = json.loads(paths["template_spec"].read_text())
    correction = spec["scenario"].get("stock_top_up_csv")
    if correction:
        paths["stock_top_up"] = (paths["template_spec"].parent / correction).resolve()
        spec["scenario"]["stock_top_up_csv"] = str(output / "stock_top_up.csv")
    spec["world"] = str(paths["world"])
    spec["scenario"]["outbound_csv"] = str(output / "outbound.csv")
    spec["scenario"]["onhand_csv"] = str(output / "onhand.csv")
    payloads = {
        "outbound.csv": csv_bytes(out_fields, selected_rows),
        "onhand.csv": csv_bytes(stock_fields, stock),
        "world_spec.json": json_bytes(spec),
    }
    if correction:
        payloads["stock_top_up.csv"] = paths["stock_top_up"].read_bytes()
    manifest = {
        "format_version": 3,
        "purpose": "full source workload" if args.orders is None else "explicit internal source-derived subset",
        "counts": counts,
        "source_order_count": len({row["order_id"] for row in rows}),
        "arrivals": args.arrivals,
        "t0": T0 if args.arrivals == "ready" else None,
        "source_stock": stock_audit(rows, stock),
        "selected_stock": stock_audit(selected_rows, stock),
        "selection": {
            "rule": "all source orders" if args.orders is None else "sort distinct source order IDs; primary[i] = IDs[floor(i*N/target)]",
            "feasibility": "preserve all demand and report deficits" if args.orders is None else "reserve every line of each accepted whole order against remaining original on-hand stock",
            "rounding": "Rust f64.max(0).round() as u32; half away from zero, saturating",
            "replacement_rule": "none" if args.orders is None else "after all primaries, scan non-primary IDs in sorted order until target is met",
            "primary_order_ids": primary,
            "replacement_order_ids": replacements,
            "rejected_candidates": rejected,
            "selected_order_ids": sorted(selected),
        },
        "preservation": {
            "outbound": "all columns and every line of selected orders; order_drop_time normalized only with --arrivals ready",
            "onhand": "all source SKUs and original quantities, independent of selected orders; explicit template top-ups copied separately",
            "configuration": "composition/settings copied from template; map unchanged; only CSV/map paths rebased",
            "stock_policy": spec["scenario"]["stock"],
        },
        "inputs": {name: {"path": str(path), "sha256": sha256(path.read_bytes())} for name, path in paths.items()},
        "generator_sha256": sha256(Path(__file__).read_bytes()),
        "outputs": {name: {"bytes": len(data), "sha256": sha256(data)} for name, data in payloads.items()},
    }
    payloads["manifest.json"] = json_bytes(manifest)
    if any((output / name).exists() for name in payloads):
        raise ValueError(f"{output}: refusing to overwrite existing run artifacts")
    if any(path in {output / name for name in payloads} for path in paths.values()):
        raise ValueError("output would overwrite a source file")
    output.mkdir(parents=True, exist_ok=True)
    for name, data in payloads.items():
        with (output / name).open("xb") as destination:
            destination.write(data)
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--orders", type=int, help="explicit internal subset size; omitted means every source order")
    parser.add_argument("--arrivals", choices=("source", "ready"), default="source",
                        help="source preserves timestamps; ready explicitly releases every selected order at T0")
    parser.add_argument("--outbound", type=Path, default=FIXTURES / "walmart-data/outbound.csv")
    parser.add_argument("--onhand", type=Path, default=FIXTURES / "walmart-data/onhand.csv")
    parser.add_argument("--world", type=Path, default=FIXTURES / "walmart_dfw6.json")
    args = parser.parse_args()
    try:
        manifest = prepare(args)
    except (ValueError, OSError, csv.Error) as error:
        parser.exit(1, f"error: {error}\n")
    print(json.dumps(manifest["counts"], indent=2, sort_keys=True))
    print(f"Replacements: {len(manifest['selection']['replacement_order_ids'])}")
    audit = manifest["selected_stock"]
    print(f"Source stock shortage: {audit['short_unit_count']} units across {audit['short_sku_count']} SKUs")
    print(f"Run spec: {args.output_dir.expanduser().resolve() / 'world_spec.json'}")


if __name__ == "__main__":
    main()
