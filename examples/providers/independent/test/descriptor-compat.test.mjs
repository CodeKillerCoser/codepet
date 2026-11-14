import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";

for (const backend of ["echo", "pi"])
  for (const version of [2, 3])
    test(`${backend} descriptor ${version}/2: explicit agent features and old/new peer negotiation`, async (t) => {
      async function checkPeer(protocol, selectedVersion) {
        const child = spawn(process.execPath, ["provider.mjs"], {
          cwd: process.env.PROVIDER_TEST_DIRECTORY || process.cwd(),
          env: {
            ...process.env,
            ECHO_BACKEND: backend,
            ECHO_PROTOCOL_VERSION: String(version),
            CODEPET_PROVIDER_TRANSPORT: "stdio-jsonrpc-v1",
            ECHO_PI_COMMAND: "/does-not-exist/fixture-pi",
          },
        });
        t.after(() => child.kill());
        let buffer = "",
          next = 0;
        const pending = new Map();
        child.stderr.resume();
        child.stdout.setEncoding("utf8");
        child.stdout.on("data", (chunk) => {
          buffer += chunk;
          for (;;) {
            const n = buffer.indexOf("\n");
            if (n < 0) break;
            const message = JSON.parse(buffer.slice(0, n));
            buffer = buffer.slice(n + 1);
            pending.get(message.id)?.(message);
            pending.delete(message.id);
          }
        });
        const rpc = (method, params = {}) =>
          new Promise((resolve, reject) => {
            const id = String(++next);
            const timeout = setTimeout(
              () => reject(Error(`${method} timeout`)),
              3000,
            );
            pending.set(id, (m) => {
              clearTimeout(timeout);
              resolve(m);
            });
            child.stdin.write(
              JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n",
            );
          });
        const response = await rpc("provider.initialize", {
          hostClientId: "fixture",
          hostDeviceId: "fixture",
          hostVersion: "fixture",
          protocol,
        });
        if (selectedVersion === undefined) {
          assert.equal(response.error?.data.code, "protocol_version_mismatch");
          assert.equal(
            (await rpc("provider.describe")).error?.data.code,
            "not_initialized",
          );
          child.kill();
          return;
        }
        assert.equal(response.error, undefined, JSON.stringify(response.error));
        assert.deepEqual(response.result.protocol, {
          version,
          minVersion: 2,
          selectedVersion,
        });
        assert.deepEqual(response.result.plugin.protocol, {
          version,
          minVersion: 2,
        });
        if (version === 3)
          assert.deepEqual(response.result.plugin.features, ["agent"]);
        else
          assert.equal(
            Object.hasOwn(response.result.plugin, "features"),
            false,
            "legacy descriptor omits new field",
          );
        const described = await rpc("provider.describe");
        assert.deepEqual(described.result.plugin, response.result.plugin);
        const inventory = await rpc("runtime.getInstalled");
        assert.equal(inventory.result.selectionRequired, false);
        // An external Node/pi executable is a process dependency, not a selectable
        // runtime inventory. Merely having event.subscribe does not claim observation.
        assert.equal(
          response.result.plugin.features?.includes("runtimeInventory") ??
            false,
          false,
        );
        assert.equal(
          response.result.plugin.features?.includes("observation") ?? false,
          false,
        );
        assert.deepEqual((await rpc("provider.shutdown")).result, {
          accepted: true,
        });
      }
      await checkPeer({ version: 2, minVersion: 2 }, 2);
      await checkPeer({ version: 3, minVersion: 2 }, version);
      await checkPeer({ version: 4, minVersion: 2 }, version);
      await checkPeer({ version: 1, minVersion: 1 }, undefined);
      await checkPeer({ version: 4, minVersion: 4 }, undefined);
      if (version === 3) await checkPeer({ version: 3, minVersion: 3 }, 3);
      else await checkPeer({ version: 3, minVersion: 3 }, undefined);
    });
