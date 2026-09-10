import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

function generate(platform, files, mode = "disabled") {
  const directory = mkdtempSync(join(tmpdir(), "codepet-release-"));
  try {
    for (const name of files) writeFileSync(join(directory, name), "fixture");
    const output = join(directory, "latest.json");
    const result = spawnSync(process.execPath, ["scripts/generate_latest_json.mjs", "--repo", "CodeKillerCoser/codepet", "--tag", "v1.0.0", "--version", "1.0.0", "--platform", platform, "--mac-updater", mode, "--artifacts", directory, "--output", output], { encoding: "utf8" });
    return { ...result, manifest: result.status === 0 ? JSON.parse(readFileSync(output, "utf8")) : null };
  } finally { rmSync(directory, { recursive: true, force: true }); }
}
test("DMG-only mac release does not require signatures or advertise an updater", () => {
  const result = generate("mac", ["Code Pet_aarch64.dmg"]);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.manifest.platforms, {});
});
test("combined release keeps the signed Windows update with DMG-only mac", () => {
  const result = generate("all", ["Code Pet_aarch64.dmg", "Code Pet_setup.exe", "Code Pet_setup.exe.sig"]);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(Object.keys(result.manifest.platforms), ["windows-x86_64"]);
  assert.match(result.manifest.platforms["windows-x86_64"].url, /Code\.Pet_setup\.exe$/);
});
test("missing DMG and missing Windows signature still fail publication", () => {
  assert.notEqual(generate("mac", []).status, 0);
  assert.notEqual(generate("all", ["Code Pet_aarch64.dmg", "Code Pet_setup.exe"]).status, 0);
});
test("legacy signed mac updater generation remains explicit and supported", () => {
  const result = generate("mac", ["Code Pet.app.tar.gz", "Code Pet.app.tar.gz.sig"], "enabled");
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(Object.keys(result.manifest.platforms), ["macos-universal"]);
});
