import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { packageIcons } from "../package-icons.mjs";
import { validatePluginManifest } from "../contracts.mjs";

test("package generator includes validated HTTPS icon for Echo/Pi and fixed business v4", (t) => {
  const temporary = fs.mkdtempSync(path.join(process.cwd(), ".package-icons-"));
  t.after(() => fs.rmSync(temporary, { recursive: true, force: true }));
  for (const backend of ["echo", "pi", "notice"]) {
    const out = path.join(temporary, backend);
    execFileSync(
      process.execPath,
      [
        "make-package.mjs",
        "--out",
        out,
        "--backend",
        backend,
        "--protocol-version",
        "4",
      ],
      { cwd: process.cwd(), stdio: "pipe" },
    );
    const manifest = JSON.parse(
      fs.readFileSync(path.join(out, "codepet-provider.json")),
    );
    assert.equal(
      validatePluginManifest(manifest),
      true,
      JSON.stringify(validatePluginManifest.errors),
    );
    assert.equal(
      manifest.icon,
      packageIcons[backend === "notice" ? "echo" : backend],
    );
    assert.equal(manifest.env.ECHO_PROTOCOL_VERSION, "4");
    if (backend === "notice") assert.deepEqual(manifest.instances, []);
    const url = new URL(manifest.icon);
    assert.equal(url.protocol, "https:");
    assert.equal(url.username, "");
    assert.equal(url.password, "");
  }
  for (const icon of [
    "http://example.com/icon.png",
    "https://user:pass@example.com/icon.png",
    "",
  ]) {
    const out = path.join(temporary, "invalid");
    assert.throws(
      () =>
        execFileSync(
          process.execPath,
          ["make-package.mjs", "--out", out, "--icon", icon],
          { cwd: process.cwd(), stdio: "pipe" },
        ),
      (error) => String(error.stderr).includes("Invalid package manifest"),
    );
    assert.equal(
      fs.existsSync(out),
      false,
      "invalid manifest is rejected before package writes",
    );
  }
});
