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
  assert.equal(sdkManifest.protocolVersion, 4);
  assert.equal(sdkManifest.sdkVersion, 2);
  assert.equal(
    methods.find((method) => method.name === "usage.query").direction,
    "hostToPlugin",
  );
  for (const retired of ["event.subscribe", "event.unsubscribe"]) {
    assert.equal(requestValidators.has(retired), false);
    assert.equal(responseValidators.has(retired), false);
  }
  assert.equal(eventValidators.has("event.notification"), false);

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
      protocol: { version: 4, minVersion: 4 },
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
    : [
        "jsonl",
        "zstd",
        "mux",
        "pi-jsonl",
        "notice-jsonl",
        "notice-zstd",
        "notice-mux",
      ].map((name) => path.join(base, "packages", name));
  for (const dir of dirs) {
    const manifest = JSON.parse(
      fs.readFileSync(path.join(dir, "codepet-provider.json")),
    );
    const icon = new URL(manifest.icon);
    assert.equal(icon.protocol, "https:");
    assert.equal(icon.username, "");
    assert.equal(icon.password, "");
    assert.equal(icon.search, "");
    const withoutIcon = { ...manifest };
    delete withoutIcon.icon;
    assert.equal(
      validatePluginManifest(withoutIcon),
      false,
      "full schema rejects missing icon for discovery, loading and import",
    );
    assert.ok(
      validatePluginManifest.errors.some(
        (e) => e.keyword === "required" && e.params.missingProperty === "icon",
      ),
    );
    const withoutTransport = { ...manifest };
    delete withoutTransport.transport;
    assert.equal(
      validatePluginManifest(withoutTransport),
      false,
      "missing transport is not synthesized",
    );
    assert.ok(
      validatePluginManifest.errors.some(
        (e) =>
          e.keyword === "required" && e.params.missingProperty === "transport",
      ),
    );
    assert.equal(
      validatePluginManifest({ ...manifest, manifestVersion: 1 }),
      false,
      "manifest v1 is not accepted",
    );
    assert.ok(
      validatePluginManifest.errors.some(
        (e) => e.instancePath === "/manifestVersion" && e.keyword === "const",
      ),
    );
    assert.equal(
      validatePluginManifest({ ...manifest, executable: "./legacy-provider" }),
      false,
      "legacy executable field is not accepted even with a valid command",
    );
    assert.ok(
      validatePluginManifest.errors.some(
        (e) =>
          e.keyword === "additionalProperties" &&
          e.params.additionalProperty === "executable",
      ),
    );
    const legacy = {
      ...manifest,
      manifestVersion: 1,
      executable: "./legacy-provider",
    };
    delete legacy.command;
    assert.equal(
      validatePluginManifest(legacy),
      false,
      "old executable-only manifest cannot bypass required command",
    );
    assert.ok(
      validatePluginManifest.errors.some(
        (e) =>
          e.keyword === "required" && e.params.missingProperty === "command",
      ),
    );
    for (const invalid of [
      "",
      null,
      42,
      "http://example.com/icon.png",
      "/icon.png",
      "data:image/png;base64,AAAA",
      "https://user:password@example.com/icon.png",
      "https://example.com/a b.png",
      "https:// example.com/icon.png",
    ]) {
      assert.equal(
        validatePluginManifest({ ...manifest, icon: invalid }),
        false,
        `invalid icon ${invalid}`,
      );
      assert.ok(
        validatePluginManifest.errors.some((e) => e.instancePath === "/icon"),
      );
    }
    assert.equal(
      validatePluginManifest(manifest),
      true,
      JSON.stringify(validatePluginManifest.errors),
    );
  }
  assert.equal(validatePluginManifest({}), false);
});

test("descriptor feature collection remains optional, combinable and unique", () => {
  const plugin = {
    pluginId: "fixture.agent",
    displayName: "Fixture",
    version: "3.0.0",
    protocol: { version: 4, minVersion: 4 },
    instanceKinds: ["fixture"],
  };
  const validate = responseValidators.get("provider.describe");
  for (const features of [
    undefined,
    [],
    ["agent"],
    ["agent", "observation"],
    ["agent", "observation", "runtimeInventory"],
  ]) {
    assert.equal(
      validate({
        plugin: { ...plugin, ...(features === undefined ? {} : { features }) },
      }),
      true,
      JSON.stringify(validate.errors),
    );
  }
  assert.equal(
    validate({ plugin: { ...plugin, features: ["agent", "agent"] } }),
    false,
  );
  assert.equal(validate({ plugin: { ...plugin, features: null } }), false);
});
