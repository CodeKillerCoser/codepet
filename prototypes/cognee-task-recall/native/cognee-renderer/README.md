# Ladybug + original Cognee renderer, without a Python runtime

This experiment imports the already processed real TaskRun visualization snapshot
into a new native Ladybug database. Node reads the database and assembles Cognee's
original HTML/JavaScript views. No Python process, Python package import or model
call participates in rendering or serving.

The original view modules and HTML template are copied without editing; their
hashes and license files are kept in the generated bundle. The assembler changes
D3 and Inter font URLs to local assets. Graph, Schema, Memory and Semantic views
use the existing processed schema/memory metadata, semantic positions and clusters.
This proves presentation can be separated from Python. It does not port Cognee's
document extraction, topology preprocessing, PCA or clustering implementation.

## Reproduce

First run the parent native build to obtain the Ladybug C++ driver/library.
Then, from the repository root:

```sh
node prototypes/cognee-task-recall/native/cognee-renderer/stage.mjs
node prototypes/cognee-task-recall/native/cognee-renderer/verify.mjs
node artifacts/cognee-task-recall-qa/ladybug-cognee-renderer/app/server.mjs artifacts/cognee-task-recall-qa/ladybug-cognee-renderer 50547
```

Staging reads frontend source files from the existing Cognee 1.6.10 installation
and the prior generated TaskRun HTML, parses known JSON assignments without
evaluating the source, and downloads D3 7.9.0 and Inter fonts once. It does not run
Python. Re-staging an existing database is rejected; pass a fresh output directory
as the first positional argument to preserve old evidence.

After staging, the generated directory contains all runtime code, native library,
database and frontend assets. It can be copied independently of the `.venv` and
the earlier experiments. It requires Node and macOS arm64; Node itself is not
bundled. Run `node app/server.mjs` inside that directory, or supply its absolute
path explicitly. Open `http://127.0.0.1:50547/`.

`verify.mjs <bundle-root>` checks full database round-trip, property update and
restoration, unchanged upstream assets, safe JSON embedding and a cold start with
an empty executable search path. The server must be stopped before running the
verification because both open a native writer handle.

## Data and limits

- Real previous corpus: 9 TaskRuns, 148 total graph nodes and 325 links, including
  112 entities and 74 semantic relationships. No synthetic nodes in the displayed
  graph and no fresh extraction call.
- Every page request reads native Ladybug records. `/graph.json` exposes the same
  read-back snapshot; `/provenance.json` identifies sources and asset hashes.
- Property changes are read back; instance and document labels resolve from current
  node records. This is not a complete property-dependency invalidation system.
- Topology-dependent schema/memory metadata and semantic layout are persisted from
  the processed snapshot. A topology mismatch is rejected rather than silently
  rendering stale structures. New document ingestion, deletes and fresh layout
  computation need their own Rust/Node processing pipeline.
- This prototype's database emphasizes visualization compatibility, with node
  attributes stored as JSON and actual native graph edges. It does not reproduce
  the parent experiment's vector/FTS indexes in this new database.
- No production MCP, Tauri integration or production task ledger changes.

Evidence in the generated bundle includes `provenance.json`, `verification.json`,
`runtime-audit.json`, `ui-qa.json` and per-view screenshots. Design and remaining
work are recorded in `knowledge/40-runbooks/ladybug-cognee-native-renderer.md`.
