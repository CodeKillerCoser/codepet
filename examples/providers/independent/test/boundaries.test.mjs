import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
function launch(t, transport = "stdio-jsonrpc-v1", env = {}) {
  const child = spawn(process.execPath, ["provider.mjs"], {
    cwd: process.env.PROVIDER_TEST_DIRECTORY || process.cwd(),
    env: { ...process.env, CODEPET_PROVIDER_TRANSPORT: transport, ...env },
  });
  child.stderr.resume();
  t.after(() => child.kill());
  return child;
}
const init = {
  hostClientId: "c",
  hostDeviceId: "d",
  hostVersion: "test",
  protocol: { version: 2, minVersion: 2 },
};
function connect(child) {
  let next = 0,
    buffer = "";
  const waiting = new Map(),
    events = [];
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk) => {
    buffer += chunk;
    for (;;) {
      const n = buffer.indexOf("\n");
      if (n < 0) break;
      const reply = JSON.parse(buffer.slice(0, n));
      buffer = buffer.slice(n + 1);
      if (reply.id) {
        waiting.get(reply.id)?.(reply);
        waiting.delete(reply.id);
      } else events.push(reply);
    }
  });
  return {
    events,
    rpc: (method, params) =>
      new Promise((resolve) => {
        const id = String(++next);
        waiting.set(id, resolve);
        child.stdin.write(
          JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
        );
      }),
  };
}
test("failed initialization does not commit; heartbeat invariants; concurrent response correlation", async (t) => {
  const child = launch(t, "stdio-jsonrpc-v1", {
      ECHO_DESCRIBE_DELAY_MS: "120",
    }),
    { rpc } = connect(child);
  assert.ok(
    (
      await rpc("provider.initialize", {
        ...init,
        protocol: { version: 1, minVersion: 1 },
      })
    ).error,
  );
  assert.equal(
    (await rpc("provider.describe", {})).error.data.code,
    "not_initialized",
  );
  assert.equal(
    (await rpc("provider.initialize", init)).result.protocol.selectedVersion,
    2,
  );
  const slow = rpc("provider.describe", {});
  const ping = {
    sequence: 1,
    hostSessionId: "one",
    clients: {
      revision: 2,
      connections: [
        { connectionId: "c1", clientId: "client" },
        { connectionId: "c2", clientId: "other" },
      ],
    },
    instances: [],
  };
  assert.equal((await rpc("provider.ping", ping)).result.sequence, 1);
  assert.ok((await rpc("provider.ping", ping)).error);
  assert.ok(
    (await rpc("provider.ping", { ...ping, sequence: 2, hostSessionId: "two" }))
      .error,
  );
  assert.ok(
    (
      await rpc("provider.ping", {
        ...ping,
        sequence: 2,
        clients: { revision: 1, connections: [] },
      })
    ).error,
  );
  assert.ok(
    (
      await rpc("provider.ping", {
        ...ping,
        sequence: 2,
        clients: { revision: 2, connections: [] },
      })
    ).error,
  );
  assert.ok(
    (
      await rpc("provider.ping", {
        ...ping,
        sequence: 2,
        clients: {
          ...ping.clients,
          connections: [...ping.clients.connections].reverse(),
        },
      })
    ).error,
    "unchanged revision rejects reordered connection snapshot",
  );
  assert.ok((await slow).result.plugin);
  await rpc("provider.shutdown", {});
});
test("turn interrupt cancels output without duplicate accepted work", async (t) => {
  const child = launch(t, "stdio-jsonrpc-v1", { ECHO_TURN_DELAY_MS: "100" }),
    { rpc, events } = connect(child);
  await rpc("provider.initialize", init);
  const route = {
    deviceId: "d",
    providerPluginId: "independent.echo.jsonl",
    providerInstanceId: "i",
  };
  await rpc("instance.create", {
    route,
    instanceKind: "independent-echo",
    displayName: "Echo",
    settings: {},
  });
  await rpc("instance.start", { route });
  const c = (
    await rpc("conversation.create", { route, permissionLevel: "default" })
  ).result.conversation;
  const conversation = {
    ...route,
    nativeResourceId: c.resource.nativeResourceId,
  };
  const request = {
    conversation,
    clientRequestId: "request",
    capabilityRevision: (await rpc("instance.capabilities", { route })).result
      .capabilities.revision,
    input: { kind: "text", text: "cancel this" },
    selection: {},
  };
  const result = (await rpc("turn.start", request)).result;
  assert.ok(
    (
      await rpc("turn.start", {
        ...request,
        input: { kind: "text", text: "changed" },
      })
    ).error,
  );
  const interrupted = await rpc("turn.interrupt", {
    conversation,
    turn: { ...route, nativeResourceId: result.turn.resource.nativeResourceId },
  });
  assert.equal(interrupted.result.turn.status, "interrupted");
  await new Promise((r) => setTimeout(r, 180));
  assert.equal(
    events.filter((e) => e.method === "event.turnOutputDelta").length,
    0,
  );
  assert.equal(
    events.findLast((e) => e.method === "event.turnUpserted").params.turn
      .status,
    "interrupted",
  );
  await rpc("provider.shutdown", {});
});
for (const [name, transport, data] of [
  ["truncated JSONL", "stdio-jsonrpc-v1", Buffer.from("{")],
  [
    "bad frame version",
    "stdio-codepet-zstd-v1",
    Buffer.from("43505246020100000000", "hex"),
  ],
  [
    "oversized declared frame",
    "stdio-codepet-zstd-v1",
    Buffer.from("435052460101ffffffff", "hex"),
  ],
  [
    "corrupt zstd",
    "stdio-codepet-zstd-v1",
    Buffer.from("4350524601010000000178", "hex"),
  ],
])
  test(name, async (t) => {
    const child = launch(t, transport),
      exit = once(child, "exit");
    child.stdin.end(data);
    assert.equal((await exit)[0], 1);
  });
test("one partial/stalled provider leaves independent provider responsive", async (t) => {
  const stalled = launch(t, "stdio-codepet-zstd-v1");
  stalled.stdin.write(Buffer.from("CPRF"));
  const healthy = launch(t);
  const { rpc } = connect(healthy);
  assert.equal(
    (await rpc("provider.initialize", init)).result.protocol.selectedVersion,
    2,
  );
  assert.equal(
    (
      await rpc("provider.ping", {
        sequence: 1,
        hostSessionId: "s",
        clients: { revision: 1, connections: [] },
        instances: [],
      })
    ).result.sequence,
    1,
  );
  await rpc("provider.shutdown", {});
});
