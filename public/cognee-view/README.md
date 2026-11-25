# Cognee native preview assets

This directory ships with webcontent. `vendor/` contains Cognee 1.6.10's original
template and view scripts, D3 7.9.0, Inter fonts, their licenses and SHA-256
provenance. It has no dependency on the experimental artifacts directory at build
or runtime. `embed.css` changes sizing only; upstream view code stays unchanged.

The Host imports `index.js` and calls
`mountCogneeGraph(element, snapshot, {signal})`. Give the element a nonzero height.
The returned view has `detail` and `destroy()`. A refresh mounts a replacement,
then destroys the previous view after successful initialization. Selection and
pan/zoom reset on refresh. Empty data is handled by the containing product page.

Input is specifically Cognee's native visualization JSON (`nodes`, `links`,
`schema_graph`, `memory_map`, `semantic_positions`, `semantic_clusters`, etc.).
This is not a universal graph protocol. All extraction, indexing, projection and
clustering happens in Cognee. No browser model calls or graph processing is added.

The renderer runs in an opaque iframe with `sandbox="allow-scripts"`. It cannot
access Host DOM/IPC. Parent/frame messages are bound to the exact Window and a
per-mount token; graph strings are JSON-escaped. Assets are loaded locally by the
Host and embedded into the frame, whose CSP disables network connections. View
history remains inside the frame rather than changing App navigation.

When upgrading upstream assets, retain license notices, refresh provenance and
run `node --test scripts/cognee_graph_view.test.mjs`, component tests, webcontent
build and browser checks of all four views. Test `GraphSnapshot` from real sidecar
output, including Semantic coordinates, and check both compact layout and error
recovery.
