# L-shape engine snapshot

`l_shape_6_aisles.json` is the current template engine's authored input at six aisles and width 54400 mm. Its document and entity identities come from the existing snapshot. The world consumed by tests is `data/snapshots/worlds/flowgraph/l_shape_6_aisles.json`, exported through WPS.

Regenerate the authored seed with:

```sh
cargo test -p site-template --test flowgraph write_l_shape_seed -- --ignored
```

This writes only the seed. Refresh its SHA256 in `data/seed/wps-manifests/l-shape-6-aisles.json`. With the validated `studio-snapshots` tool from the WPS/data prerequisite and a fresh WPS store containing Foundry's registered application definitions, capture the export:

```sh
STUDIO_WPS_URL=http://127.0.0.1:<isolated-port> studio-snapshots \
  --manifest data/seed/wps-manifests/l-shape-6-aisles.json \
  --report data/seed/migration-evidence/worlds-current/simulator-world-source-l-shape-6-aisles/receipt.json \
  --commit <source-commit> --seed
```

The snapshot tool publishes an output only after the strict source comparison passes. It is supplied by the separate WPS/data work; this bounded follow-up does not add that tooling or change the application registry. Do not edit the snapshot by hand.

The engine comparison uses identity order for entity declarations, matching WPS export, and removes only `doc.assembled_from`. Every component value, nested array, relationship and authored comment remains exact.

The receipt and provenance beside it record the fresh WPS changeset, export/source hashes, frozen runtime and registry hashes, and engine inputs. The retired legacy fixture is preserved by commit `92f4f281e56a27df2275860b9728997ba1e6b782`; `originals.json` records its byte hash. Its 24 authored changes are exclusively the two updated robot definitions. Against the previous WPS snapshot, the current engine removes the template-card root comment and corrects the D30 constant-mode cruise comment; no physical values or geometry change. The recorded old-to-engine deltas separate these source updates from the zero-difference WPS round trip. `doc.assembled_from` is WPS provenance, not authored input.
