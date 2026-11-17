# Requirements Document

> Copied from the workspace Kiro spec on 2026-10-11 and edited here, so the repo copy is the one that matches the code. Deltas from the original: `Position`'s `Debug` form is `pos(N)` and `DistanceDelta`'s is `delta(N)`, with no paste-back claim; `DistanceDelta::magnitude()` is exact at `i64::MIN`; `Position::checked_offset` has no `i64::MIN` exclusion. Added after code review on 2026-10-14: clauses 3.11, 3.12 and 5.9; the `mm` constructor and `as_millimeter` accessor were renamed `millimeter` / `as_millimeter` at a reviewer's request.

## Introduction

The `desunits` Rust crate is the shared vocabulary layer for the GreyOrange
discrete-event simulator stack. It currently holds two modules: `simtime` (the
simulation clock vocabulary) and `ids` (domain identifier newtypes). Both
`dessim` and `desmap` re-export or depend on it.

Distance quantities are present throughout the codebase as raw `u64`/`i64`/`f64`
bindings carrying `_mm`, `_m`, or implicit-unit suffixes. A survey of the two
primary crates identifies the following classes of raw bindings:

- **`desmap`**: `cell_size_mm: f64`, `distance_mm: u64`, `zone_height_mm: u64`,
  `zone_width_mm: Option<u64>`, `x_mm: f64`, `y_mm: f64`
- **`dessim` pathfinder**: `speed_mm_per_sec: u64`, `cell_spacing_mm: u64`,
  `accel_distance_mm: f64`, `cruise_distance_mm: f64`, `total_distance_mm: f64`
- **VTM motion model**: `RACK_HEIGHT_M: f64`, `BUFFER_HEIGHT_M: f64`,
  `h_dist_to_rack: f64` (implicit meters)
- **Robot configs**: `speed: f64` (documented in m/s, no suffix)

Three concrete bug classes follow from this:

1. **Silent unit confusion.** `desmap` edges store `distance_mm: u64` (integer
   millimeters) while the VTM motion model works in `f64` meters. A call site
   that passes a millimeter value where a meter value is expected, or vice versa,
   compiles silently and produces a 1000× error.

2. **No non-negativity enforcement.** Graph edge distances and zone dimensions
   are non-negative by definition, but they are stored as plain `u64` with no
   type-level distinction from a signed displacement. A signed `i64` displacement
   (e.g., a position correction) can be assigned to a `u64` field via `as u64`
   with wrapping, and no compiler diagnostic fires.

3. **Unsigned/signed confusion at displacement sites.** The difference of two
   positions (e.g., `current_x_mm - origin_x_mm`) is signed but stored as
   `f64` with ad-hoc sign handling, with no typed home for a signed
   displacement.

This spec covers the correctness requirements for three distance primitive types
to be added to `desunits` as a new `distance` module (`Position`, `Distance`,
and `DistanceDelta`), analogous to how `simtime` provides `SimTime`,
`SimDuration`, and `SimOffset`.

Migration of existing raw bindings throughout `dessim` and `desmap` is explicitly
**out of scope** for this spec and is a separate effort.

## Glossary

- **Position**: An absolute coordinate along a single axis, backed by `u64`
  millimeters. Analogous to `SimTime`. Used for world coordinates, cell
  positions, shelf heights measured from a fixed origin.
- **Distance**: An unsigned span between two positions, backed by `u64`
  millimeters. Analogous to `SimDuration`. Used for edge lengths, cell sizes,
  zone dimensions: any non-negative length.
- **DistanceDelta**: A signed displacement, backed by `i64` millimeters.
  Analogous to `SimOffset`. Used for position corrections, offsets, error
  magnitudes with a direction.
- **Distance_Module**: The `sim/desunits/src/distance.rs` module. Zero crate
  dependencies beyond `std` and `serde`; must not depend on `simtime`, `ids`,
  or any other crate.
- **Raw binding**: A field, local, or function parameter typed as `u64`, `i64`,
  or `f64` whose name carries a `_mm`, `_m`, or implicit-unit suffix indicating
  a bare distance quantity without type-level unit information.
- **Inverted subtraction**: `Position - Position` or `Distance - Distance` where
  the right operand exceeds the left. Panics rather than saturating, because
  overflow in distance arithmetic is always a bug, not a sentinel condition.
- **Tier_1**: `cargo test --release -p desunits --lib` (fast, no external
  dependencies).
- **Tier_1b**: `cargo test --release -p dessim --lib`, the full `dessim` test
  suite, run to confirm that adding the `distance` module to `desunits` does not
  break any existing behavior.

---

## Requirements

### Requirement 1: Position primitive type contract

**User Story:** As a simulation engineer, I want a type-safe absolute-position
type for a single axis, so that unit errors between millimeters and meters are
caught at compile time rather than silently producing 1000× wrong answers.

#### Acceptance Criteria

1. THE Distance_Module SHALL define `Position` as a newtype over `u64` millimeters with a private field.
2. THE Distance_Module SHALL provide `Position::ZERO`, `Position::MAX` (equal to `u64::MAX`), and `Position::millimeter(u64)` as the only construction paths.
3. THE Distance_Module SHALL provide `Position::as_millimeter(self) -> u64` as the only extraction path, making the unit explicit at every call site that reads back the raw integer value.
4. WHEN `Position + Distance` would overflow `u64::MAX`, THE Distance_Module SHALL panic with a message indicating a position overflow; there is no saturating variant.
5. WHEN `Position - Distance` would underflow below `u64::MIN`, THE Distance_Module SHALL panic with a message indicating a position underflow.
6. THE Distance_Module SHALL provide `Position::checked_add(self, d: Distance) -> Option<Position>` that returns `None` on overflow as a non-panicking alternative to the operator.
7. THE Distance_Module SHALL provide `Position::checked_sub(self, d: Distance) -> Option<Position>` that returns `None` on underflow as a non-panicking alternative to the operator.
8. WHEN `Position - Position` is computed and the left operand is greater than or equal to the right operand, THE Distance_Module SHALL return a `Distance` equal to the unsigned difference.
9. WHEN `Position - Position` is computed and the left operand is less than the right operand, THE Distance_Module SHALL panic with a message indicating the operands are inverted.
10. THE Distance_Module SHALL provide `Position::abs_diff(self, other: Position) -> Distance` that is order-independent and never panics, for call sites that only need the magnitude of a position difference without caring which is larger.
11. THE Distance_Module SHALL provide `Position::diff(self, other: Position) -> DistanceDelta` that returns a positive `DistanceDelta` when `self > other`, a negative `DistanceDelta` when `self < other`, zero when equal, and saturates to `i64::MAX` or `i64::MIN` when the unsigned gap exceeds the signed range.
12. THE Distance_Module SHALL serialize `Position` as a bare `u64` integer in both JSON and MessagePack wire formats, and SHALL reject deserialization of any value that is not a non-negative integer or that exceeds `u64::MAX` (including floating-point representations and negative integers) with a deserialization error.
13. THE Distance_Module SHALL implement `Debug` for `Position` in the form `pos(N)` where `N` is the raw millimeter integer value.
14. THE Distance_Module SHALL implement `Display` for `Position` such that: whole-meter values (millimeter count divisible by 1000) render as `Nm` with no decimal point (e.g., `0m`, `5m`); all other values render as `N.Xm`, `N.XXm`, or `N.XXXm` using the minimum number of decimal places required so that no trailing zeros appear after the last significant digit (e.g., `1.5m`, `1.38m`, `0.001m`).

---

### Requirement 2: Distance primitive type contract

**User Story:** As a simulation engineer, I want a type-safe unsigned-span type,
so that negative-distance bugs are caught at compile time and distances cannot be
confused with signed displacements.

#### Acceptance Criteria

1. THE Distance_Module SHALL define `Distance` as a newtype over `u64` millimeters with a private field.
2. THE Distance_Module SHALL provide `Distance::ZERO`, `Distance::MAX` (equal to `u64::MAX`), and `Distance::millimeter(u64)` as construction paths.
3. THE Distance_Module SHALL provide `Distance::as_millimeter(self) -> u64` as the only extraction path.
4. WHEN `Distance + Distance` would overflow `u64::MAX`, THE Distance_Module SHALL panic with a message indicating a distance overflow.
5. WHEN `Distance - Distance` would underflow below zero, THE Distance_Module SHALL panic with a message indicating a distance underflow; there is no saturating variant.
6. THE Distance_Module SHALL provide `Distance::checked_add(self, rhs: Distance) -> Option<Distance>` that returns `None` on overflow.
7. THE Distance_Module SHALL provide `Distance::checked_sub(self, rhs: Distance) -> Option<Distance>` that returns `None` on underflow.
8. THE Distance_Module SHALL provide `Distance::abs_diff(self, other: Distance) -> Distance` such that `a.abs_diff(b)` equals `b.abs_diff(a)` for all values, never panics, and `d.abs_diff(d)` equals `Distance::ZERO` for all `d`.
9. THE Distance_Module SHALL provide `Distance * u64 -> Distance` and `Distance / u64 -> Distance`, where multiplication overflow SHALL panic with a message indicating a distance overflow, and division by zero SHALL panic with a message indicating division by zero; and SHALL provide `Distance::checked_mul(self, rhs: u64) -> Option<Distance>` that returns `None` on overflow and `Distance::checked_div(self, rhs: u64) -> Option<Distance>` that returns `None` when `rhs` is zero, as non-panicking alternatives.
10. THE Distance_Module SHALL serialize and deserialize `Distance` as a bare `u64` integer in both JSON and MessagePack wire formats, and SHALL reject deserialization of values that cannot be represented as a `u64` with a deserialization error.
11. THE Distance_Module SHALL implement `Debug` for `Distance` in the form `d(N)` where `N` is the raw millimeter integer value.
12. THE Distance_Module SHALL implement `Display` for `Distance` in the form `Nm` for whole-meter values (e.g., `5m`) and `N.Xm` / `N.XXm` / `N.XXXm` using the fewest decimal places needed to represent the value exactly (e.g., `1.500m` is written as `1.5m`, `0.001m` for one millimeter).

---

### Requirement 3: DistanceDelta primitive type contract

**User Story:** As a simulation engineer, I want a signed displacement type, so
that position corrections, offsets, and error quantities have a typed home
instead of leaking as raw `i64` or sign-ambiguous `f64`.

#### Acceptance Criteria

1. THE Distance_Module SHALL define `DistanceDelta` as a newtype over `i64` millimeters with a private field.
2. THE Distance_Module SHALL provide `DistanceDelta::millimeter(i64)`, `DistanceDelta::ZERO`, `DistanceDelta::is_negative() -> bool`, `DistanceDelta::magnitude() -> Distance`, and negation, where: `is_negative()` returns `true` if and only if the internal `i64` value is strictly less than zero; `magnitude()` returns a `Distance` equal to the absolute value of the internal `i64` millimeters, exact for every value including `DistanceDelta::millimeter(i64::MIN)` (2^63 mm, which `u64` holds); negation returns a `DistanceDelta` with the internal value negated, with `i64::MIN` saturating to `DistanceDelta::millimeter(i64::MAX)` rather than wrapping.
3. THE Distance_Module SHALL provide `DistanceDelta::as_millimeter(self) -> i64` as the only extraction path.
4. THE Distance_Module SHALL implement `From<Distance> for DistanceDelta`, mapping an unsigned `Distance` of `n` millimeters to `DistanceDelta::millimeter(n as i64)`, and IF the `Distance` value exceeds `i64::MAX` millimeters, THEN THE Distance_Module SHALL saturate to `DistanceDelta::millimeter(i64::MAX)`.
5. WHEN `Position + DistanceDelta` is computed and the result would be less than `Position::ZERO` (i.e., the delta is negative and its magnitude exceeds the position), THE Distance_Module SHALL panic with a message indicating the result is below zero.
6. WHEN `Position + DistanceDelta` is computed and the result would exceed the maximum value of the `Position` type, THE Distance_Module SHALL panic with a message indicating a position overflow.
7. THE Distance_Module SHALL provide `Position::checked_offset(self, delta: DistanceDelta) -> Option<Position>` as a non-panicking alternative to `Position + DistanceDelta`, returning `None` on either underflow or overflow.
8. THE Distance_Module SHALL serialize `DistanceDelta` as a bare `i64` integer in both JSON and MessagePack wire formats, and SHALL deserialize a bare `i64` integer in both formats back to the equivalent `DistanceDelta` with identical internal value.
9. THE Distance_Module SHALL implement `Debug` for `DistanceDelta` in the form `delta(-N)` for negative values and `delta(N)` for non-negative values, where `N` is the absolute raw millimeter integer value.
10. THE Distance_Module SHALL implement `Display` for `DistanceDelta` using the same adaptive-precision meters format as `Distance`, prefixed with `-` for negative values and no prefix for zero or positive values.
11. THE Distance_Module SHALL give `Position`, `Distance`, and `DistanceDelta` `Debug` forms that are distinct from one another, so that a rendered value identifies its type without surrounding context; a unit test SHALL pin all three forms.
12. THE Distance_Module SHALL treat `Debug` output as a diagnostic rendering only, and SHALL NOT describe it as paste-back-capable unless a test evaluates the rendered text back to the originating value.

---

### Requirement 4: Wire format stability

**User Story:** As a simulation engineer, I want the wire format of all three
primitive types to be pinned by tests, so that a field type change from a raw
integer to one of these types never silently changes the serialized
representation and breaks stored map files or event streams.

#### Acceptance Criteria

1. THE Distance_Module SHALL include tests that assert `Position::millimeter(1000)`, `Distance::millimeter(1000)`, and `DistanceDelta::millimeter(-500)` each serialize to the corresponding bare integer JSON values `1000`, `1000`, and `-500` respectively (not wrapped objects such as `{"mm": 1000}`).
2. THE Distance_Module SHALL include tests that assert `Position::millimeter(1000)`, `Distance::millimeter(1000)`, and `DistanceDelta::millimeter(-500)` each serialize to their corresponding bare integer values in the MessagePack compact integer encoding, matching the encoding of the equivalent raw primitives `1000u64`, `1000u64`, and `-500i64`.
3. THE Distance_Module SHALL include tests that assert JSON round-trip fidelity for `Position` using the values `0`, `1`, `1000`, and `u64::MAX`, verifying that `deserialize(serialize(p)).as_millimeter() == p.as_millimeter()` for each value.
4. THE Distance_Module SHALL include tests that assert JSON round-trip fidelity for `Distance` using the values `0`, `1`, `1000`, and `u64::MAX`, verifying that `deserialize(serialize(d)).as_millimeter() == d.as_millimeter()` for each value.
5. THE Distance_Module SHALL include tests that assert JSON round-trip fidelity for `DistanceDelta` using the values `0`, `1`, `-1`, `500`, `-500`, `i64::MAX`, and `i64::MIN`, verifying that `deserialize(serialize(delta)).as_millimeter() == delta.as_millimeter()` for each value.
6. THE Distance_Module SHALL include tests that assert MessagePack round-trip fidelity for `Position`, `Distance`, and `DistanceDelta` using the same representative value sets defined in criteria 3, 4, and 5 respectively, verifying that `deserialize(serialize(v)).as_millimeter() == v.as_millimeter()` for each value.
7. IF a JSON input contains a wrapped-object representation (e.g., a JSON object rather than a bare integer) where a `Position`, `Distance`, or `DistanceDelta` value is expected, THEN the Distance_Module SHALL return a deserialization error indicating an unexpected type.

---

### Requirement 5: Arithmetic correctness properties

**User Story:** As a simulation engineer, I want the arithmetic contract of the
distance primitives to be verified by property tests, so that overflow/underflow
policies, commutativity, and round-trip properties are confirmed correct over the
full value range and not just at selected examples.

#### Acceptance Criteria

1. THE Distance_Module SHALL ensure that FOR ALL `Position` values `p` (typed as a `u64`-backed mm newtype) and `Distance` values `d` (typed as a `u64`-backed mm newtype) where `d.as_millimeter() <= p.as_millimeter()`, `(p - d + d).as_millimeter()` equals `p.as_millimeter()` (round-trip through subtraction and addition).
2. THE Distance_Module SHALL ensure that FOR ALL `Position` values `p1` and `p2`, `p1.abs_diff(p2).as_millimeter()` equals `p2.abs_diff(p1).as_millimeter()` (commutativity of abs_diff).
3. THE Distance_Module SHALL ensure that FOR ALL `Position` values `p`, `p.abs_diff(p).as_millimeter()` equals `0` (reflexivity of abs_diff).
4. THE Distance_Module SHALL ensure that FOR ALL `Distance` values `d`, `d.abs_diff(d).as_millimeter()` equals `0`.
5. THE Distance_Module SHALL ensure that FOR ALL `Distance` values `d1` and `d2` where `d1.as_millimeter() >= d2.as_millimeter()`, `(d1 - d2 + d2).as_millimeter()` equals `d1.as_millimeter()` (round-trip through subtraction and addition).
6. THE Distance_Module SHALL ensure that FOR ALL `Distance` values `d` and non-zero `u64` values `n` where `d.as_millimeter().checked_mul(n).is_some()`, `(d * n / n).as_millimeter()` equals `d.as_millimeter()`, where `/` is integer division truncating toward zero (round-trip through multiplication and integer division).
7. THE Distance_Module SHALL ensure that FOR ALL `DistanceDelta` values `delta`, `(-(-delta)).as_millimeter()` equals `delta.as_millimeter()` (double-negation identity), except when `delta == DistanceDelta::millimeter(i64::MIN)`, where the result SHALL equal `DistanceDelta::millimeter(-i64::MAX).as_millimeter()` (the first negation saturates to `i64::MAX`, and the second negates that value).
8. THE Distance_Module SHALL ensure that FOR ALL `Distance` values `d` where `d.as_millimeter() <= i64::MAX as u64`, the `DistanceDelta` produced by `DistanceDelta::from(d)` satisfies `result.as_millimeter() == d.as_millimeter() as i64`.
9. THE Distance_Module SHALL ensure that FOR ALL operands where a `checked_*` method (`Position::checked_add`, `Position::checked_sub`, `Position::checked_offset`, `Distance::checked_add`, `Distance::checked_sub`, `Distance::checked_mul`, `Distance::checked_div`) returns `Some(v)`, the corresponding panicking operator applied to the same operands returns a value equal to `v`; and that FOR ALL `Position` values `p1` and `p2` where `p1.as_millimeter() >= p2.as_millimeter()`, `(p1 - p2).as_millimeter()` equals `p1.as_millimeter() - p2.as_millimeter()`. The property test SHALL compare against the operator, not only against raw integer arithmetic.

---

### Requirement 6: Module placement and dependency discipline

**User Story:** As a maintainer, I want the distance primitives to live in
`desunits` alongside `simtime`, so that any crate depending on `desunits` for
time types gets the distance types at the same cost, and no new dependency edges
are introduced.

#### Acceptance Criteria

1. THE `desunits` crate SHALL add a `distance` module at `sim/desunits/src/distance.rs` and expose it as `pub mod distance` from `sim/desunits/src/lib.rs`.
2. THE Distance_Module SHALL depend only on `std` and `serde`; it SHALL NOT import from `crate::simtime`, `crate::ids`, or any external crate beyond those two, and the `distance` module's entry in `sim/desunits/Cargo.toml` SHALL introduce zero new workspace-internal dependency edges.
3. THE `desunits` crate SHALL re-export `Position`, `Distance`, and `DistanceDelta` at the crate root so call sites can write `desunits::Distance` without naming the submodule.
4. WHEN `cargo test --release -p desunits --lib` is run after the module is added, THE `desunits` crate SHALL pass all tests with no failures and with zero compiler warnings whose source file path falls within `sim/desunits/src/distance.rs`.
5. WHEN `cargo test --release -p dessim --lib` is run after `desunits` is updated, THE `dessim` crate SHALL pass all pre-existing tests with a count equal to or greater than the count recorded on the commit immediately prior to the change.

---

### Requirement 7: Future extension note: speed and acceleration types

**User Story:** As a maintainer, I want the known future extension points
documented alongside the primitives, so that future contributors do not
independently invent incompatible speed or acceleration types before a unified
design is agreed.

#### Acceptance Criteria

1. THE Distance_Module SHALL include a module-level documentation comment that identifies `Speed` (a `Distance` per `SimDuration`) and `Acceleration` (a `Speed` per `SimDuration`) as deliberately deferred types, and notes that they will require `f64` backing because they involve division of integer quantities whose result is not generally an integer.
2. THE Distance_Module SHALL include a module-level documentation comment that identifies a `DistanceRange` type (a half-open interval `[start: Position, end: Position)`, analogous to `TimeRange`) as a deliberately deferred type, noting that the invariant-enforcement pattern from `TimeRange` (private fields, fallible construction, validated deserialization) should be carried forward when it is introduced.
3. THE Distance_Module SHALL NOT define stub implementations, empty enum variants, or placeholder types for `Speed`, `Acceleration`, or `DistanceRange`; documentation comments are sufficient.
4. IF the `distance` module source contains any of the identifiers `Speed`, `Acceleration`, or `DistanceRange` outside of documentation comments (e.g., in `struct`, `enum`, `type`, `fn`, `impl`, or `const` definitions), THEN the `desunits` crate SHALL fail to compile or a dedicated lint test SHALL fail, making the no-stub rule mechanically verifiable.
5. THE Distance_Module SHALL include a module-level documentation comment that identifies `quantize_floor` (grid-snapping a distance to the nearest multiple of a resolution) as a deliberately deferred method on `Distance`, noting that it is useful for cell-grid alignment and slot-pitch snapping but has no confirmed current use case.

