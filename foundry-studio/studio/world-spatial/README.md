# Foundry spatial arithmetic

`Steps`, `Degrees` and `centre_of` define Foundry application geometry for WASM
generators. Whole frame steps, exact quarter turns, half-up rounding and overflow
refusal are application choices. WPS's generic generator SDK supplies no spatial
units or placement policy.

The implementation and all eight original tests were preserved from
`world-proto/generator/sdk/src/units.rs` at commit `2e62d4c`; only the module
charter changed.

Run `cargo test --manifest-path studio/world-spatial/Cargo.toml` from the Foundry
root. The rack, stamp and tiling WASM callers are tested through
`studio/world-generators/check.py`.
