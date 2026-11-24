# Codex project contracts

Snapshots come from official `codex app-server generate-json-schema --experimental` exports, retaining complete ProjectListParams/ProjectListResponse and their local definitions. The shared Project DTO is identical across project/list, project/read, project/create, and project/update for each captured version.

- Legacy baseline: **0.166.0-alpha.5.1**. Downloaded official npm `@openai/codex@0.166.0-alpha.5.1-darwin-arm64`; verified the registry SHA512 integrity before extracting and running schema export. This is the user-selected old implementation upper bound, not an exhaustive claim about every older binary.
- Current adaptation: **0.171.0-alpha.13**, exported from the locally installed ChatGPT bundled CLI. Project.position was removed; project/list documents returned order as authoritative. Latest per-interface branches also serve newer versions, which are not claimed verified.

For the next upgrade, export both schemas in development and compare:

```sh
"$BASELINE_CODEX" app-server generate-json-schema --experimental --out /tmp/codex-baseline-schema
"$CURRENT_CODEX" app-server generate-json-schema --experimental --out /tmp/codex-current-schema
python3 scripts/diff_codex_schema.py /tmp/codex-baseline-schema /tmp/codex-current-schema > /tmp/codex-schema-diff.json
```

The script ignores descriptive annotations, reports nested definitions, and emits export fingerprints. Review changed requests, responses, events and defaults against the fields actually consumed; a structural diff is not a semantic compatibility verdict. Capture stable exports too when evaluating a feature that is not experimental. Never start an old binary against the user's current database just to obtain its schema.

Only adjust dispatch and compatibility implementations for affected interfaces. Keep Provider/Gateway, common mapped output types, native project IDs and thread/start.projectId stable. Verification and install evidence: `knowledge/40-runbooks/codex-project-position-schema-drift.md`.
