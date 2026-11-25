# @codepet/cognee-graph-view

Independent browser adapter for Cognee 1.6.10's original Graph, Schema, Memory and
Semantic views. It takes a visualization snapshot JSON object and mounts the
upstream UI in an isolated iframe. The browser module has no Node, Python,
database, model or npm runtime dependencies. D3 and fonts are static local assets.

```js
import { mountCogneeGraph } from './src/index.mjs';

const view = await mountCogneeGraph(document.querySelector('#graph'), snapshot);
await view.update(nextSnapshot);
view.destroy();
```

Give the host element a height. Deploy `src/`, `frame.html` and `assets/` together
under the same origin as the host app. `assetsURL` can override the default asset
directory. Updates replace the iframe and reset its selection, pan and zoom; await
each update before starting another. Upstream view globals and DOM IDs stay inside
the iframe, and do not become globals of the host page.

Input requires a nonempty `nodes` array with unique string `id`s, and `links` whose `source`/`target`
reference those IDs. The adapter accepts the previous prototype's camelCase payload
or Cognee's `color_maps`, `schema_graph`, `memory_map`, `search_events`,
`semantic_positions` and `semantic_clusters` fields. Full Schema, Memory and
Semantic views require their corresponding processed input. Missing input uses the
upstream empty state; this package does not extract entities, build indexes,
recompute a schema, project embeddings or cluster nodes.
Use `destroy()` to clear a view when no graph data is available.

## Build and try

From the repository root, after the previous native renderer experiment is staged:

```sh
node prototypes/cognee-graph-view/scripts/prepare.mjs
node prototypes/cognee-graph-view/scripts/serve.mjs
```

Open `http://127.0.0.1:50548/example/index.html`. The static example reads a copied
real TaskRun snapshot; it does not contact Ladybug or the previous Node service.
The development static server can be replaced by any static host. The generated
package is under `artifacts/cognee-task-recall-qa/cognee-graph-view/` and can be
copied separately. No package is published to a registry.

The build step copies upstream assets byte-for-byte, verifies their recorded
hashes, and retains upstream licenses/notices. Only generated page assembly and
font URL resolution are adapted. This is an experimental extracted adapter, not
an upstream-maintained standalone Cognee npm package. Chromium is verified;
production Svelte/Tauri integration and WebView policies remain to be tested.
