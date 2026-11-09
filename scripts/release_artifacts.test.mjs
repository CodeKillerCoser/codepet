import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { artifactNames, buildMetadata, stageArtifacts, validateArtifacts, validateBuildRun, validateMetadata } from "./release_artifacts.mjs";
const commitSha = "a68ec0c99818d7d9d42b8c1d6728826c0042e939";
const metadata = platform => buildMetadata({ baseVersion: "0.3.10-beta", commitSha, platform, runId: "12345" });
function fixture(fn) {
  const dir = mkdtempSync(join(tmpdir(), "codepet-build-"));
  try { return fn(dir); } finally { rmSync(dir, { recursive: true, force: true }); }
}
function stage(dir, m, platform) {
  const input = join(dir, platform), output = join(dir, "output");
  mkdirSync(input, { recursive: true }); mkdirSync(output, { recursive: true });
  const source = platform === "macos-arm64" ? `Code Pet_${m.version}_aarch64.dmg` : `Code Pet_${m.version}_x64-setup.exe`;
  writeFileSync(join(input, source), "signed-installer-bytes");
  if (platform === "windows-x86_64") writeFileSync(join(input, `${source}.sig`), "updater-signature");
  const target = stageArtifacts(m, platform, input, output);
  assert.equal(readFileSync(join(output, target), "utf8"), "signed-installer-bytes");
  writeFileSync(join(output, artifactNames(m).metadata), JSON.stringify(m));
  return { input, output, source, target };
}

test("direct DMG and Windows names include platform, base version and source commit", () => {
  assert.deepEqual(artifactNames(metadata("all")), {
    metadata: "codepet-build-all_0.3.10-beta_a68ec0c.json",
    "macos-arm64": "codepet-macos-arm64_0.3.10-beta_a68ec0c.dmg",
    "windows-x86_64": "codepet-windows-x86_64_0.3.10-beta_a68ec0c-setup.exe",
  });
  assert.throws(() => validateMetadata({ ...metadata("all"), version: "0.3.9-beta+other" }), /metadata/);
  assert.throws(() => buildMetadata({ ...metadata("all"), commitSha: "a68ec0c" }), /full/);
});
for (const selection of ["all", "mac", "win"]) {
  test(`${selection} build round trips into publishable assets with exact updater URLs`, () => fixture(dir => {
    const m = metadata(selection);
    if (selection !== "win") stage(dir, m, "macos-arm64");
    if (selection !== "mac") stage(dir, m, "windows-x86_64");
    const output = join(dir, "output");
    assert.deepEqual(validateArtifacts(output, "12345"), m);
    const manifest = join(dir, "latest.json");
    const result = spawnSync(process.execPath, ["scripts/generate_latest_json.mjs", "--repo", "CodeKillerCoser/codepet", "--tag", m.tag, "--version", m.version, "--platform", selection, "--mac-updater", "disabled", "--artifacts", output, "--output", manifest], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    const json = JSON.parse(readFileSync(manifest, "utf8"));
    assert.deepEqual(Object.keys(json.platforms), selection === "mac" ? [] : ["windows-x86_64"]);
    if (selection !== "mac") {
      assert.equal(json.platforms["windows-x86_64"].signature, "updater-signature");
      assert.equal(json.platforms["windows-x86_64"].url, `https://github.com/CodeKillerCoser/codepet/releases/download/${encodeURIComponent(m.tag)}/${artifactNames(m)["windows-x86_64"]}`);
    }
    if (selection === "mac") assert.equal(readdirSync(output).filter(name => !name.endsWith(".json"))[0].endsWith(".dmg"), true);
  }));
}

test("missing, extra, empty, or mixed-build assets and wrong run IDs fail publication", () => fixture(dir => {
  const m = metadata("all");
  const { output } = stage(dir, m, "macos-arm64");
  assert.throws(() => validateArtifacts(output, "12345"), /Missing/);
  stage(dir, m, "windows-x86_64");
  assert.throws(() => validateArtifacts(output, "54321"), /run ID/);
  writeFileSync(join(output, "old.dmg"), "stale");
  assert.throws(() => validateArtifacts(output, "12345"), /mixed-build/);
  rmSync(join(output, "old.dmg"));
  writeFileSync(join(output, artifactNames(m)["macos-arm64"]), "");
  assert.throws(() => validateArtifacts(output, "12345"), /Empty/);
}));

test("staging rejects ambiguous installers, wrong versions and missing signatures", () => fixture(dir => {
  const m = metadata("win"), { input, output, source } = stage(dir, m, "windows-x86_64");
  rmSync(join(input, `${source}.sig`));
  assert.throws(() => stageArtifacts(m, "windows-x86_64", input, output));
  writeFileSync(join(input, "Old_0.1.0_x64-setup.exe"), "old");
  assert.throws(() => stageArtifacts(m, "windows-x86_64", input, output), /Expected one/);
  rmSync(join(input, source));
  assert.throws(() => stageArtifacts(m, "windows-x86_64", input, output), /version and architecture/);
  assert.throws(() => stageArtifacts(m, "macos-arm64", input, output), /not selected/);
}));

test("only completed successful Build runs from this repository can be released", () => {
  const repo = "CodeKillerCoser/codepet";
  const run = { id: 12345, repository: { full_name: repo }, path: ".github/workflows/build.yml", event: "workflow_dispatch", status: "completed", conclusion: "success" };
  validateBuildRun(run, "12345", repo);
  for (const change of [{ id: 54321 }, { path: ".github/workflows/release.yml" }, { event: "pull_request" }, { status: "in_progress" }, { conclusion: "failure" }, { conclusion: "cancelled" }, { repository: { full_name: "another/repo" } }]) {
    assert.throws(() => validateBuildRun({ ...run, ...change }, "12345", repo), /successful completed Build/);
  }
});
