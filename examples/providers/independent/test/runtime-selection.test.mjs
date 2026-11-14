import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";

for (const backend of ["echo", "pi"]) {
  test(`${backend} owns dependencies without Host runtime selection`, async (t) => {
    const child = spawn(process.execPath, ["provider.mjs"], {
      cwd: process.env.PROVIDER_TEST_DIRECTORY || process.cwd(),
      env: {
        ...process.env,
        ECHO_BACKEND: backend,
        CODEPET_PROVIDER_TRANSPORT: "stdio-jsonrpc-v1",
        // Deliberately nonexistent: runtime metadata must neither start pi nor
        // claim the dependency is installed. Actual conversation startup owns it.
        ECHO_PI_COMMAND: "/nonexistent/independent-provider-pi",
      },
    });
    t.after(() => child.kill());
    let buffer = Buffer.alloc(0);
    let next = 0;
    const pending = new Map();
    child.stdout.on("data", (chunk) => {
      buffer = Buffer.concat([buffer, chunk]);
      for (;;) {
        const newline = buffer.indexOf(10);
        if (newline < 0) break;
        const message = JSON.parse(buffer.subarray(0, newline));
        buffer = buffer.subarray(newline + 1);
        const waiter = pending.get(message.id);
        if (!waiter) continue;
        pending.delete(message.id);
        waiter.resolve(message);
      }
    });
    const rpc = (method, params = {}) =>
      new Promise((resolve, reject) => {
        const id = `runtime-${++next}`;
        const timeout = setTimeout(
          () => reject(Error(`${method} timeout`)),
          3000,
        );
        pending.set(id, {
          resolve: (reply) => {
            clearTimeout(timeout);
            resolve(reply);
          },
        });
        child.stdin.write(
          JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
        );
      });
    const rejected = await rpc("provider.initialize", {
      hostClientId: "test",
      hostDeviceId: "test",
      hostVersion: "test",
      protocol: { version: 1, minVersion: 1 },
    });
    assert.equal(rejected.error?.data.code, "protocol_version_mismatch");
    const initialized = await rpc("provider.initialize", {
      hostClientId: "test",
      hostDeviceId: "test",
      hostVersion: "test",
      protocol: { version: 4, minVersion: 2, futureOffer: true },
      futureField: true,
    });
    assert.equal(initialized.error, undefined);
    assert.deepEqual(initialized.result.protocol, {
      version: 2,
      minVersion: 2,
      selectedVersion: 2,
    });
    assert.deepEqual(initialized.result.plugin.protocol, {
      version: 2,
      minVersion: 2,
    });
    const unknown = await rpc("future.method", {});
    assert.equal(unknown.error?.code, -32601);

    for (const params of [{}, { refresh: true }]) {
      const response = await rpc("runtime.getInstalled", params);
      assert.equal(response.error, undefined);
      assert.deepEqual(response.result, {
        selectionRequired: false,
        harnessList: [],
        selected: null,
        scanning: false,
      });
      assert.equal(Object.hasOwn(response.result, "scanError"), false);
    }
    const exited = once(child, "exit");
    assert.deepEqual((await rpc("provider.shutdown")).result, {
      accepted: true,
    });
    assert.equal((await exited)[0], 0);
  });
}
