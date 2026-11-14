import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { Duplex } from "node:stream";
import { zstdDecompressSync } from "node:zlib";
import { Client } from "@bbk47/yamux";
import { Reader, write, bootstrapFrame, limits, features } from "../mux.mjs";
test("mux public Yamux interoperability, grant, FIN, ordered event ACK", async (t) => {
  const child = spawn(process.execPath, ["provider.mjs"], {
    cwd: process.env.PROVIDER_TEST_DIRECTORY || process.cwd(),
    env: {
      ...process.env,
      CODEPET_PROVIDER_TRANSPORT: "stdio-codepet-mux-v1",
      ECHO_TURN_DELAY_MS: "2",
    },
  });
  t.after(() => child.kill());
  let stderr = "";
  child.stderr.on("data", (d) => (stderr += d));
  const bootstrap = new Reader(child.stdout);
  await write(
    child.stdin,
    bootstrapFrame(1, { supportedVersions: [1], features, receive: limits }),
  );
  let h = await bootstrap.read(8);
  assert.equal(h[5], 2);
  assert.equal(
    JSON.parse((await bootstrap.read(h.readUInt16BE(6))).toString())
      .selectedVersion,
    1,
  );
  await write(child.stdin, bootstrapFrame(3, 1));
  h = await bootstrap.read(8);
  assert.equal(h[5], 4);
  await bootstrap.read(h.readUInt16BE(6));
  child.stdout.pause();
  const remaining = bootstrap.detach();
  const transport = new Duplex({
    read() {
      child.stdout.resume();
    },
    write(b, e, cb) {
      child.stdin.write(b, cb);
    },
  });
  child.stdout.on("data", (d) => {
    if (!transport.push(d)) child.stdout.pause();
  });
  child.stdout.on("end", () => transport.push(null));
  const client = Client(transport, {
    initialStreamWindow: 262144,
    maxFrameSize: 16384,
  });
  client.on("error", () => {});
  if (remaining.length) transport.push(remaining);
  child.stdout.resume();
  let next = 0;
  const received = [];
  const readMessage = async (stream, reader, expectedClass) => {
    const env = await reader.read(10);
    assert.equal(env[0], 1);
    if (expectedClass !== undefined)
      assert.equal(
        env[1],
        expectedClass,
        "response selects its own class unless control",
      );
    await write(stream, Buffer.from([1]));
    const f = await reader.read(env.readUInt32BE(2));
    assert.equal(f.toString("ascii", 0, 4), "CPRF");
    const raw =
      f[5] === 1 ? zstdDecompressSync(f.subarray(10)) : f.subarray(10);
    assert.equal(raw.length, env.readUInt32BE(6));
    return JSON.parse(raw);
  };
  client.on("stream", (stream) => {
    stream.on("error", () => {});
    const r = new Reader(stream);
    void (async () => {
      received.push(await readMessage(stream, r));
      await write(stream, Buffer.from([2]));
      stream.end();
      r.detach();
    })();
  });
  const rpc = async (method, params, responseClass, expectedError) => {
    const stream = client.openStream();
    stream.on("error", () => {});
    const reader = new Reader(stream);
    const id = String(++next),
      body = Buffer.from(
        JSON.stringify({ jsonrpc: "2.0", id, method, params }),
      );
    const frame = Buffer.alloc(10 + body.length);
    frame.write("CPRF");
    frame[4] = 1;
    frame[5] = 0;
    frame.writeUInt32BE(body.length, 6);
    body.copy(frame, 10);
    const env = Buffer.alloc(10);
    env[0] = 1;
    env[1] = ["provider.ping", "provider.shutdown"].includes(method)
      ? 2
      : body.length <= limits.smallMessageBytes &&
          frame.length <= limits.smallMessageBytes
        ? 1
        : 0;
    env.writeUInt32BE(frame.length, 2);
    env.writeUInt32BE(body.length, 6);
    await write(stream, env);
    assert.equal((await reader.read(1))[0], 1);
    await write(stream, frame);
    const reply = await readMessage(stream, reader, responseClass);
    stream.end();
    reader.detach();
    assert.equal(reply.id, id);
    if (expectedError) {
      assert.equal(reply.error?.data.code, expectedError);
      return reply.error;
    }
    assert.equal(reply.error, undefined, JSON.stringify(reply.error));
    return reply.result;
  };
  const baseInit = { hostClientId: "c", hostDeviceId: "d", hostVersion: "t" };
  for (const protocol of [
    { version: 1, minVersion: 1 },
    { version: 3, minVersion: 3 },
  ])
    await rpc(
      "provider.initialize",
      { ...baseInit, protocol },
      undefined,
      "protocol_version_mismatch",
    );
  await rpc(
    "provider.initialize",
    { ...baseInit, protocol: { version: 1, minVersion: 2 } },
    undefined,
    "invalid_protocol_range",
  );
  await rpc(
    "provider.initialize",
    { ...baseInit, protocol: { version: 0, minVersion: 0 } },
    undefined,
    "invalid_request",
  );
  await rpc("provider.describe", {}, undefined, "not_initialized");
  const initialized = await rpc("provider.initialize", {
    hostClientId: "c",
    hostDeviceId: "d",
    hostVersion: "t",
    protocol: { version: 4, minVersion: 2, futureOffer: true },
    futureInitialize: true,
  });
  assert.deepEqual(initialized.protocol, {
    version: 2,
    minVersion: 2,
    selectedVersion: 2,
  });
  assert.deepEqual(initialized.plugin.protocol, { version: 2, minVersion: 2 });
  assert.equal(
    (await rpc("future.unknown", {}, undefined, "method_not_found")).code,
    -32601,
  );
  await rpc("provider.describe", { futureDescribe: true });
  assert.deepEqual(await rpc("runtime.getInstalled", { refresh: true }), {
    selectionRequired: false,
    harnessList: [],
    selected: null,
    scanning: false,
  });
  const route = {
    deviceId: "d",
    providerPluginId: "independent.echo.jsonl",
    providerInstanceId: "mux-test",
  };
  await rpc("instance.create", {
    route,
    instanceKind: "independent-echo",
    displayName: "Echo",
    settings: { chunkSize: 4096 },
  });
  await rpc("instance.start", { route });
  const caps = await rpc("instance.capabilities", { route });
  const created = await rpc("conversation.create", {
    route,
    permissionLevel: "default",
  });
  const expectedSelection = {
    accessModeId: "default",
    model: { kind: "flat", modelId: "echo" },
  };
  assert.deepEqual(created.conversation.selection, expectedSelection);
  const selectionConversation = {
    ...route,
    nativeResourceId: created.conversation.resource.nativeResourceId,
  };
  assert.deepEqual(
    (
      await rpc("conversation.acquireInteraction", {
        conversation: selectionConversation,
      })
    ).selection,
    expectedSelection,
  );
  for (const selection of [
    { accessModeId: "missing" },
    { model: { kind: "flat", modelId: "missing" } },
    { reasoningEffortId: "high" },
  ])
    for (let attempt = 0; attempt < 2; attempt++)
      await rpc(
        "turn.start",
        {
          conversation: selectionConversation,
          clientRequestId: "invalid-choice",
          capabilityRevision: caps.capabilities.revision,
          input: { kind: "text", text: "rejected" },
          selection,
        },
        undefined,
        "invalid_selection",
      );
  const first = await rpc("turn.start", {
    conversation: {
      ...route,
      nativeResourceId: created.conversation.resource.nativeResourceId,
    },
    clientRequestId: "once",
    capabilityRevision: caps.capabilities.revision,
    input: { kind: "text", text: "mux 世界" },
    selection: {},
  });
  assert.deepEqual(first.effectiveSelection, expectedSelection);
  await new Promise((r) => setTimeout(r, 100));
  assert.equal(
    received
      .filter((e) => e.method === "event.turnOutputDelta")
      .map((e) => e.params.delta)
      .join(""),
    "mux 世界",
  );
  assert.equal(received.at(-1).method, "event.conversationUpserted");
  await rpc(
    "provider.ping",
    {
      sequence: 1,
      hostSessionId: "mux-test",
      clients: { revision: 1, connections: [] },
      instances: [route],
    },
    2,
  );
  const largeText = "L".repeat(4095) + "🙂" + "中文🙂".repeat(14000);
  const conversation = {
    ...route,
    nativeResourceId: created.conversation.resource.nativeResourceId,
  };
  const second = await rpc(
    "turn.start",
    {
      conversation,
      clientRequestId: "large-history",
      capabilityRevision: caps.capabilities.revision,
      input: { kind: "text", text: largeText },
      selection: {},
    },
    0,
  );
  assert.deepEqual(second.effectiveSelection, expectedSelection);
  const deadline = Date.now() + 3000;
  while (
    !received.some(
      (e) =>
        e.method === "event.turnUpserted" &&
        e.params.turn.resource.nativeResourceId ===
          second.turn.resource.nativeResourceId &&
        e.params.turn.status === "completed",
    )
  ) {
    if (Date.now() > deadline) throw Error("large turn completion timeout");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  const unicodeDeltas = received
    .filter(
      (e) =>
        e.method === "event.turnOutputDelta" &&
        e.params.turn.nativeResourceId ===
          second.turn.resource.nativeResourceId,
    )
    .map((e) => e.params.delta);
  assert.equal(unicodeDeltas.join(""), largeText);
  assert.ok(unicodeDeltas.every((delta) => delta.isWellFormed()));
  assert.ok(unicodeDeltas.length < 32);
  // This request is small, but its full-history response must use the normal lane.
  const history = await rpc("conversation.get", { conversation }, 0);
  assert.equal(history.items.at(-1).contents[0].text, largeText);
  const latest = await rpc("conversation.get", { conversation, limit: 1 }, 0);
  assert.equal(latest.items.length, 2);
  assert.ok(
    latest.items.every(
      (i) => i.turn.nativeResourceId === second.turn.resource.nativeResourceId,
    ),
  );
  assert.ok(latest.pageInfo.nextCursor);
  const earlier = await rpc("conversation.get", {
    conversation,
    limit: 1,
    cursor: latest.pageInfo.nextCursor,
  });
  assert.equal(earlier.items.length, 2);
  assert.ok(
    earlier.items.every(
      (i) => i.turn.nativeResourceId === first.turn.resource.nativeResourceId,
    ),
  );
  assert.equal(earlier.pageInfo.nextCursor, undefined);
  assert.equal(earlier.items[1].contents[0].text, "mux 世界");
  assert.deepEqual(
    (await rpc("conversation.acquireInteraction", { conversation })).selection,
    expectedSelection,
  );
  await rpc("provider.shutdown", {}, 2);
  assert.equal(stderr, "");
});
