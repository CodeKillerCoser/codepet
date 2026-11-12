import { test } from "node:test";
import assert from "node:assert/strict";
import { PiSession } from "../pi.mjs";
test("pi public RPC adapter allowlists configured model and text events", async (t) => {
  const pi = new PiSession({
    cwd: process.cwd(),
    command: process.execPath,
    args: ["test/fake-pi.mjs"],
  });
  t.after(() => pi.close());
  assert.deepEqual(await pi.state(), { model: "fixture/config-selected" });
  let text = "";
  await pi.run("pi 世界\u2028\n", (delta) => {
    text += delta;
  });
  assert.equal(text, "pi 世界\u2028\n");
  await pi.abort();
});
