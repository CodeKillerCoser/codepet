# Native recall engine comparison

Isolated macOS arm64 evaluation of sqlite-vec + FTS5 + SQL adjacency, CozoDB,
LadybugDB, and HelixDB against the existing Cognee TaskRun snapshot. No production
ledger access. All downloads, databases, model receipts and generated reports stay
under the already ignored `artifacts/cognee-task-recall-qa/` tree.

The compiled drivers link native C/C++/Rust libraries. Node is the evaluation
orchestrator and the Helix binding host. Python is used only for one-time export
from the previous experimental LanceDB; it is not required by these engines.
All alternatives reuse the exact extracted graph. This does not compare automatic
entity-extraction quality between frameworks.

From the repository root:

```sh
node prototypes/cognee-task-recall/native/build.mjs

artifacts/cognee-task-recall-qa/.venv/bin/python prototypes/cognee-task-recall/native/prepare.py artifacts/cognee-task-recall-qa/runs/20261124T195156701196Z artifacts/cognee-task-recall-qa/native-comparison
```

For a fresh machine, install Ladybug's `vector` and `fts` extensions once by
running the compiled `ladybug-driver` against a scratch database and sending
`{"sql":"INSTALL vector"}` / `{"sql":"INSTALL fts"}` as JSON lines on stdin.
The evaluator loads them on each new connection. They must be included in an
eventual product's pinned native dependencies; the global cache used here is
only experimental. `measure.mjs` currently names this machine's extension paths.

Keep the previously configured MLX endpoint at `127.0.0.1:11234` running for the
embedding inputs not already in `fixture.json`. No fresh LLM call is made.

```sh
node prototypes/cognee-task-recall/native/compare.mjs
node prototypes/cognee-task-recall/native/measure.mjs
cp prototypes/cognee-task-recall/native/report.html artifacts/cognee-task-recall-qa/native-comparison/index.html
node prototypes/cognee-task-recall/native/serve.mjs
```

`compare.mjs <evidence-root> [sqlite cozo ladybug helix]` can rerun selected engines;
each attempt gets a separate directory, with the latest result per engine merged
into `comparison.json`. Cozo 0.7.6 intentionally reports a failed old-keyword update
check and records its full-index-rebuild workaround. Do not turn that failure into
a claimed pass. Indexing timings reuse embeddings and extraction, so cannot be
compared to Cognee's full model-processing elapsed time.

The report at `http://127.0.0.1:50546/` visualizes actual database exports, with
entity inspection, per-Run evidence filtering, source documents and JSON download.
The graph's existence does not establish factual correctness of model output.

Checks:

```sh
node --test prototypes/cognee-task-recall/native/recall.test.mjs
python3 -m unittest discover -s prototypes/cognee-task-recall -p 'test_*.py'
npm run check:source-size
git diff --check
```

Full conclusions, size boundaries and remaining production work are in
`knowledge/10-architecture/native-task-run-recall-evaluation.md`.
