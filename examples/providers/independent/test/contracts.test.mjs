import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import {
  sdkManifest,
  methods,
  events,
  requestValidators,
  responseValidators,
  eventValidators,
  validateHello,
  validatePluginManifest,
} from "../contracts.mjs";

test("canonical SDK manifest resolves every registry reference across schema modules", () => {
  assert.equal(sdkManifest.formatVersion, 3);
  assert.equal(requestValidators.size, methods.length);
  assert.equal(responseValidators.size, methods.length);
  assert.equal(eventValidators.size, events.length);
  for (const validator of [
    ...requestValidators.values(),
    ...responseValidators.values(),
    ...eventValidators.values(),
  ])
    assert.equal(typeof validator, "function");
  const lifecycle = methods.find((m) => m.name === "provider.initialize");
  assert.match(lifecycle.request.$ref, /lifecycle\.schema\.json#/);
  assert.match(
    methods.find((m) => m.name === "conversation.get").response.$ref,
    /messages\.schema\.json#/,
  );
  assert.equal(requestValidators.get("provider.initialize")({}), false);
  assert.equal(
    requestValidators.get("provider.initialize")({
      hostClientId: "client",
      hostDeviceId: "device",
      hostVersion: "test",
      supportedVersions: { minVersion: 1, maxVersion: 1 },
    }),
    true,
  );
  assert.equal(
    responseValidators.get("runtime.getInstalled")({
      selectionRequired: false,
      harnessList: [],
      selected: null,
      scanning: false,
    }),
    true,
  );
  assert.equal(
    responseValidators.get("conversation.get")({ conversation: {}, items: [] }),
    false,
  );
  assert.equal(eventValidators.get("event.turnOutputDelta")({}), false);
  assert.equal(validateHello({}), false);
});

test("plugin installation manifests validate independently from communication schemas", () => {
  const base = process.env.PROVIDER_TEST_DIRECTORY || process.cwd();
  const dirs = process.env.PROVIDER_TEST_DIRECTORY
    ? [base]
    : ["jsonl", "zstd", "mux", "pi-jsonl"].map((name) =>
        path.join(base, "packages", name),
      );
  for (const dir of dirs) {
    const manifest = JSON.parse(
      fs.readFileSync(path.join(dir, "codepet-provider.json")),
    );
    assert.equal(
      validatePluginManifest(manifest),
      true,
      JSON.stringify(validatePluginManifest.errors),
    );
  }
  assert.equal(validatePluginManifest({}), false);
});
