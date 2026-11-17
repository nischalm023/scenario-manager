# Foundry application document tools

```text
cargo run -p foundry-world-tools --bin foundry-world -- expand scenario.world.json --out expanded.json
cargo run -p foundry-world-tools --bin foundry-world -- flatten scenario.world.json --out authored.json
cargo run -p foundry-world-tools --bin foundry-world -- adapter describe module.wasm
cargo run -p foundry-world-tools --bin foundry-world -- adapter project module.wasm scenario.world.json --out output
cargo run -p foundry-world-tools --bin foundry-world -- adapter ingest module.wasm scenario.world.json --from output
```

`expand` uses Foundry's native reader and emits its canonical expanded form.
`flatten` preserves the previous client command's authored-document behavior:
merge layers and write declarations, derived overrides and authored suppressions
for WPS import. Exclusion-generated suppressions remain generated behavior.
These commands keep the current authored format; the separate format migration
changes document metadata and its storage contract.

The adapter wrapper preserves the existing application convention: catalog
layers supply pinned entity scopes, other layers belong to the scenario's scope,
and a module requesting derived entities receives the native expansion. The
generic WPS command instead takes explicitly materialized Site rows. Both use
the same generic adapter host and file-output implementation.

This package owns local application composition. WPS remains the service for
registered schemas/modules, import/export, history and generic execution.

Tests reuse the preserved authored Walmart layer closure under
`data/seed/application-tests/lenses`. No generated oracle is described as a WPS
snapshot.
