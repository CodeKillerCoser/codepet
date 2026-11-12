import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { normalizeSelection } from "../selection.mjs";

test("selection defaults and rejection include unavailable and undeclared dimensions", () => {
  const caps = {
    accessMode: {
      defaultId: "default",
      options: [
        { id: "default", displayName: "Default" },
        { id: "blocked", displayName: "Blocked", enabled: false },
      ],
    },
    modelCatalog: {
      kind: "flat",
      models: [
        { id: "echo", displayName: "Echo" },
        { id: "blocked", displayName: "Blocked", enabled: false },
      ],
      defaultSelection: { kind: "flat", modelId: "echo" },
    },
  };
  assert.deepEqual(normalizeSelection(caps, {}), {
    accessModeId: "default",
    model: { kind: "flat", modelId: "echo" },
  });
  for (const selection of [
    { accessModeId: "missing" },
    { accessModeId: "blocked" },
    { reasoningEffortId: "high" },
    { model: { kind: "flat", modelId: "missing" } },
    { model: { kind: "flat", modelId: "blocked" } },
    { model: { kind: "grouped", providerId: "other", modelId: "echo" } },
  ])
    assert.throws(
      () => normalizeSelection(caps, selection),
      (e) => e.protocolCode === "invalid_selection",
    );
  assert.deepEqual(normalizeSelection({}, {}), {});
  assert.throws(
    () => normalizeSelection({}, { model: { kind: "flat", modelId: "echo" } }),
    (e) => e.protocolCode === "invalid_selection",
  );
});

for (const [backend, profile] of [
  ["echo", "stdio-jsonrpc-v1"],
  ["echo", "stdio-codepet-zstd-v1"],
  ["pi", "stdio-jsonrpc-v1"],
])
  test(`${backend} ${profile}: declared defaults, acquire/turn consistency and earlier history page`, async (t) => {
    let fakeDirectory;
    const env = {
      ...process.env,
      ECHO_BACKEND: backend,
      CODEPET_PROVIDER_TRANSPORT: profile,
      ECHO_PLUGIN_ID: `independent.${backend}.jsonl`,
      ECHO_TURN_DELAY_MS: "1",
      ECHO_CHUNK_SIZE: "4096",
    };
    if (backend === "pi") {
      fakeDirectory = fs.mkdtempSync(
        path.join(process.cwd(), ".pi-selection-"),
      );
      const executable = path.join(fakeDirectory, "fake-pi.mjs");
      fs.writeFileSync(
        executable,
        `#!${process.execPath}\n` +
          fs.readFileSync(path.join(process.cwd(), "test/fake-pi.mjs"), "utf8"),
        { mode: 0o755 },
      );
      env.ECHO_PI_COMMAND = executable;
    }
    const child = spawn(process.execPath, ["provider.mjs"], {
      cwd: process.env.PROVIDER_TEST_DIRECTORY || process.cwd(),
      env,
    });
    t.after(() => {
      child.kill();
      if (fakeDirectory)
        fs.rmSync(fakeDirectory, { recursive: true, force: true });
    });
    let buffer = Buffer.alloc(0),
      next = 0,
      stderr = "";
    const pending = new Map(),
      events = [];
    child.stderr.on("data", (d) => (stderr += d));
    child.stdout.on("data", (chunk) => {
      buffer = Buffer.concat([buffer, chunk]);
      for (;;) {
        let raw;
        if (profile === "stdio-jsonrpc-v1") {
          const newline = buffer.indexOf(10);
          if (newline < 0) break;
          raw = buffer.subarray(0, newline);
          buffer = buffer.subarray(newline + 1);
        } else {
          if (buffer.length < 10) break;
          const length = 10 + buffer.readUInt32BE(6);
          if (buffer.length < length) break;
          raw = zlib.zstdDecompressSync(buffer.subarray(10, length));
          buffer = buffer.subarray(length);
        }
        const message = JSON.parse(raw);
        if (message.id) {
          pending.get(message.id)?.(message);
          pending.delete(message.id);
        } else events.push(message);
      }
    });
    const exchange = (method, params) =>
      new Promise((resolve, reject) => {
        const id = String(++next);
        const timer = setTimeout(
          () => reject(Error(`${method} timeout: ${stderr}`)),
          3000,
        );
        pending.set(id, (m) => {
          clearTimeout(timer);
          resolve(m);
        });
        const raw = Buffer.from(
          JSON.stringify({ jsonrpc: "2.0", id, method, params }),
        );
        let frame;
        if (profile === "stdio-jsonrpc-v1")
          frame = Buffer.concat([raw, Buffer.from("\n")]);
        else {
          const body = zlib.zstdCompressSync(raw);
          frame = Buffer.alloc(10 + body.length);
          frame.write("CPRF");
          frame[4] = frame[5] = 1;
          frame.writeUInt32BE(body.length, 6);
          body.copy(frame, 10);
        }
        child.stdin.write(frame);
      });
    const rpc = async (method, params) => {
      const response = await exchange(method, params);
      assert.equal(response.error, undefined, JSON.stringify(response.error));
      return response.result;
    };
    const rejected = async (method, params) => {
      const response = await exchange(method, params);
      assert.equal(response.error?.data.code, "invalid_selection");
    };
    await rpc("provider.initialize", {
      hostClientId: "test",
      hostDeviceId: "test",
      hostVersion: "test",
      supportedVersions: { minVersion: 1, maxVersion: 1 },
    });
    const route = {
      deviceId: "test",
      providerPluginId: env.ECHO_PLUGIN_ID,
      providerInstanceId: "selection-test",
    };
    await rpc("instance.create", {
      route,
      instanceKind: backend === "pi" ? "independent-pi" : "independent-echo",
      displayName: "Selection test",
      settings: {},
    });
    await rpc("instance.start", { route });
    const { capabilities } = await rpc("instance.capabilities", { route });
    const expected =
      backend === "echo"
        ? { accessModeId: "default", model: { kind: "flat", modelId: "echo" } }
        : { accessModeId: "default" };
    assert.deepEqual(normalizeSelection(capabilities.turnSend, {}), expected);
    assert.deepEqual(
      capabilities.conversationCreate.selection,
      capabilities.turnSend,
    );
    await rejected("conversation.create", {
      route,
      permissionLevel: "missing",
    });
    await rejected("conversation.create", {
      route,
      permissionLevel: "default",
      reasoningEffort: "high",
    });
    await rejected("conversation.create", {
      route,
      permissionLevel: "default",
      model: "missing",
    });
    const { conversation: created } = await rpc("conversation.create", {
      route,
      permissionLevel: "default",
      ...(backend === "pi" ? { workspaceRoot: fakeDirectory } : {}),
    });
    assert.deepEqual(created.selection, expected);
    if (backend === "pi")
      assert.equal(created.model, "fixture/config-selected");
    const conversation = {
      ...route,
      nativeResourceId: created.resource.nativeResourceId,
    };
    assert.deepEqual(
      (await rpc("conversation.acquireInteraction", { conversation }))
        .selection,
      expected,
    );
    const base = { conversation, capabilityRevision: capabilities.revision };
    for (const selection of [
      { accessModeId: "missing" },
      { reasoningEffortId: "high" },
      { model: { kind: "flat", modelId: "missing" } },
    ]) {
      const invalid = {
        ...base,
        clientRequestId: "retry-invalid",
        input: { kind: "text", text: "not accepted" },
        selection,
      };
      await rejected("turn.start", invalid);
      await rejected("turn.start", invalid);
    }
    async function startTurn(clientRequestId, text, selection) {
      const accepted = await rpc("turn.start", {
        ...base,
        clientRequestId,
        input: { kind: "text", text },
        selection,
      });
      assert.deepEqual(accepted.effectiveSelection, expected);
      const end = Date.now() + 3000;
      while (
        !events.some(
          (e) =>
            e.method === "event.turnUpserted" &&
            e.params.turn.resource.nativeResourceId ===
              accepted.turn.resource.nativeResourceId &&
            e.params.turn.status === "completed",
        )
      ) {
        if (Date.now() > end) throw Error("turn completion timeout");
        await new Promise((resolve) => setTimeout(resolve, 5));
      }
      return accepted.turn;
    }
    const first = await startTurn("retry-invalid", "first turn", {});
    assert.deepEqual(
      (await rpc("conversation.acquireInteraction", { conversation }))
        .selection,
      expected,
    );
    const secondText =
      backend === "echo"
        ? "中".repeat(4095) + "🙂" + "中文🙂".repeat(4000)
        : "second turn";
    const second = await startTurn("second", secondText, expected);
    const secondDeltas = events
      .filter(
        (e) =>
          e.method === "event.turnOutputDelta" &&
          e.params.turn.nativeResourceId === second.resource.nativeResourceId,
      )
      .map((e) => e.params.delta);
    assert.equal(secondDeltas.join(""), secondText);
    assert.ok(
      secondDeltas.every((delta) => delta.isWellFormed()),
      "each delta independently contains valid Unicode",
    );
    if (backend === "echo")
      assert.ok(
        secondDeltas.length < 8,
        "large Chinese echo uses bounded event count with manifest chunk size",
      );
    const latest = await rpc("conversation.get", { conversation, limit: 1 });
    assert.deepEqual(latest.conversation.selection, expected);
    assert.equal(
      latest.items.length,
      2,
      "limit counts native turns, not items",
    );
    assert.ok(
      latest.items.every(
        (i) => i.turn.nativeResourceId === second.resource.nativeResourceId,
      ),
    );
    assert.equal(typeof latest.pageInfo.nextCursor, "string");
    assert.ok(latest.pageInfo.nextCursor.length);
    const reordered = Object.fromEntries(
      Object.entries(conversation).reverse(),
    );
    const earlier = await rpc("conversation.get", {
      conversation: reordered,
      limit: 1,
      cursor: latest.pageInfo.nextCursor,
    });
    assert.equal(earlier.items.length, 2);
    assert.ok(
      earlier.items.every(
        (i) => i.turn.nativeResourceId === first.resource.nativeResourceId,
      ),
    );
    assert.equal(earlier.pageInfo.nextCursor, undefined);
    assert.equal(earlier.items[1].contents[0].text, "first turn");
    assert.equal(latest.items[1].contents[0].text, secondText);
    assert.deepEqual(
      (await rpc("conversation.acquireInteraction", { conversation }))
        .selection,
      expected,
    );
    await rpc("provider.shutdown", {});
    assert.equal(stderr, "");
  });
