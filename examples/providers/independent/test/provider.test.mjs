import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import zlib from "node:zlib";
for (const profile of ["stdio-jsonrpc-v1", "stdio-codepet-zstd-v1"])
  test(profile, async (t) => {
    const child = spawn(process.execPath, ["provider.mjs"], {
      cwd: process.env.PROVIDER_TEST_DIRECTORY || process.cwd(),
      env: {
        ...process.env,
        CODEPET_PROVIDER_TRANSPORT: profile,
        ECHO_TURN_DELAY_MS: "2",
        ECHO_DESCRIBE_DELAY_MS: "80",
      },
    });
    t.after(() => child.kill());
    let stderr = "";
    child.stderr.on("data", (d) => (stderr += d));
    let b = Buffer.alloc(0),
      next = 1;
    const waiting = new Map(),
      events = [];
    const finish = (msg) => {
      if (msg.id) {
        const w = waiting.get(msg.id);
        assert.ok(w, "matching ID");
        waiting.delete(msg.id);
        msg.error
          ? w.reject(Error(JSON.stringify(msg.error)))
          : w.resolve(msg.result);
      } else events.push(msg);
    };
    child.stdout.on("data", (c) => {
      b = Buffer.concat([b, c]);
      for (;;) {
        if (profile === "stdio-jsonrpc-v1") {
          let i = b.indexOf(10);
          if (i < 0) break;
          finish(JSON.parse(b.subarray(0, i)));
          b = b.subarray(i + 1);
        } else {
          if (b.length < 10) break;
          let n = b.readUInt32BE(6) + 10;
          if (b.length < n) break;
          finish(JSON.parse(zlib.zstdDecompressSync(b.subarray(10, n))));
          b = b.subarray(n);
        }
      }
    });
    const rpc = (method, params) =>
      new Promise((resolve, reject) => {
        const id = String(next++);
        waiting.set(id, { resolve, reject });
        const raw = Buffer.from(
          JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
        );
        let out = raw;
        if (profile !== "stdio-jsonrpc-v1") {
          let body = zlib.zstdCompressSync(raw);
          out = Buffer.alloc(10 + body.length);
          out.write("CPRF");
          out[4] = out[5] = 1;
          out.writeUInt32BE(body.length, 6);
          body.copy(out, 10);
        }
        child.stdin.write(out.subarray(0, 3));
        child.stdin.write(out.subarray(3));
      });
    const route = {
      deviceId: "d",
      providerPluginId: "independent.echo.jsonl",
      providerInstanceId: "third-party",
    };
    const init = await rpc("provider.initialize", {
      hostClientId: "c",
      hostDeviceId: "d",
      hostVersion: "test",
      supportedVersions: { minVersion: 1, maxVersion: 1 },
    });
    assert.equal(init.selectedVersion, 1);
    assert.deepEqual(await rpc("runtime.getInstalled", { refresh: true }), {
      selectionRequired: false,
      harnessList: [],
      selected: null,
      scanning: false,
    });
    const slow = rpc("provider.describe", {});
    const pong = await rpc("provider.ping", {
      sequence: 1,
      hostSessionId: "s",
      clients: { revision: 1, connections: [] },
      instances: [],
    });
    assert.equal(pong.sequence, 1);
    await slow;
    await rpc("instance.create", {
      route,
      instanceKind: "independent-echo",
      displayName: "Echo",
      settings: {},
    });
    await rpc("instance.start", { route });
    const caps = await rpc("instance.capabilities", { route });
    const c = await rpc("conversation.create", {
      route,
      permissionLevel: "default",
    });
    const resource = {
      ...route,
      nativeResourceId: c.conversation.resource.nativeResourceId,
    };
    const request = {
      conversation: resource,
      clientRequestId: "once",
      capabilityRevision: caps.capabilities.revision,
      input: { kind: "text", text: "hello 世界\u2028newline\nend" },
      selection: {},
    };
    const started = await rpc("turn.start", request);
    const duplicate = await rpc("turn.start", request);
    assert.equal(
      duplicate.turn.resource.nativeResourceId,
      started.turn.resource.nativeResourceId,
    );
    await new Promise((r) => setTimeout(r, 100));
    assert.equal(
      events
        .filter((e) => e.method === "event.turnOutputDelta")
        .map((e) => e.params.delta)
        .join(""),
      request.input.text,
    );
    const history = await rpc("conversation.get", { conversation: resource });
    assert.equal(history.items.length, 2);
    assert.equal(history.items[1].contents[0].text, request.input.text);
    await rpc("provider.shutdown", {});
    await new Promise((r) => child.on("exit", r));
    assert.equal(stderr, "");
  });
