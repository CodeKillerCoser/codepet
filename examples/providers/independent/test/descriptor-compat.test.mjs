import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
for (const backend of ["echo", "pi", "notice"])
  test(`${backend} only supports business 4/4 and declares actual features`, async (t) => {
    const child = spawn(process.execPath, ["provider.mjs"], {
      cwd: process.env.PROVIDER_TEST_DIRECTORY || process.cwd(),
      env: {
        ...process.env,
        ECHO_BACKEND: backend,
        CODEPET_PROVIDER_TRANSPORT: "stdio-jsonrpc-v1",
        NOTICE_AUTOPUBLISH: "0",
        ECHO_PI_COMMAND: "/does-not-exist/fixture-pi",
      },
    });
    t.after(() => child.kill());
    child.stderr.resume();
    let next = 0,
      buffer = "";
    const pending = new Map();
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      buffer += chunk;
      for (;;) {
        const n = buffer.indexOf("\n");
        if (n < 0) break;
        const m = JSON.parse(buffer.slice(0, n));
        buffer = buffer.slice(n + 1);
        pending.get(m.id)?.(m);
        pending.delete(m.id);
      }
    });
    const rpc = (method, params = {}) =>
      new Promise((resolve, reject) => {
        const id = String(++next),
          timer = setTimeout(() => reject(Error("timeout")), 3000);
        pending.set(id, (m) => {
          clearTimeout(timer);
          resolve(m);
        });
        child.stdin.write(
          JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
        );
      });
    const base = { hostClientId: "t", hostDeviceId: "t", hostVersion: "t" };
    for (const protocol of [
      { version: 2, minVersion: 2 },
      { version: 3, minVersion: 2 },
      { version: 5, minVersion: 5 },
    ])
      assert.equal(
        (await rpc("provider.initialize", { ...base, protocol })).error?.data
          .code,
        "protocol_version_mismatch",
      );
    const response = await rpc("provider.initialize", {
      ...base,
      protocol: { version: 6, minVersion: 4 },
    });
    assert.equal(response.error, undefined, JSON.stringify(response.error));
    assert.deepEqual(response.result.protocol, {
      version: 4,
      minVersion: 4,
      selectedVersion: 4,
    });
    assert.deepEqual(response.result.plugin.features, [
      backend === "notice" ? "observation" : "agent",
    ]);
    assert.deepEqual(response.result.plugin.protocol, {
      version: 4,
      minVersion: 4,
    });
    if (backend === "notice")
      assert.deepEqual(response.result.plugin.instanceKinds, []);
    assert.deepEqual(
      (await rpc("provider.describe")).result.plugin,
      response.result.plugin,
    );
    assert.equal(
      (await rpc("runtime.getInstalled")).result.selectionRequired,
      false,
    );
    await rpc("provider.shutdown");
  });
