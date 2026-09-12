import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, existsSync, symlinkSync } from "node:fs";
import { join, resolve, dirname, basename } from "node:path";
import { tmpdir } from "node:os";
import { createManifest, installWebcontent, validateWebcontent } from "./webcontent.mjs";

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "codepet-webcontent-"));
  t.after(() => {
    // Only remove the exact temporary directory allocated by this test.
    if (dirname(resolve(root)) !== resolve(tmpdir()) || !basename(root).startsWith("codepet-webcontent-")) throw new Error("Invalid test directory");
    rmSync(root, { recursive: true, force: true });
  });
  const source = join(root, "source");
  mkdirSync(join(source, "assets"), { recursive: true });
  writeFileSync(join(source, "index.html"), '<script type="module" src="/assets/main.js"></script>');
  writeFileSync(join(source, "pet.html"), "<h1>桌宠</h1>");
  writeFileSync(join(source, "assets/main.js"), "console.log('UI v1')");
  createManifest(source, "1.0.0");
  return { root, source, data: join(root, "data") };
}

test("builds a verifiable package with both entry points and hashes", (t) => {
  const { source } = fixture(t);
  const manifest = validateWebcontent(source);
  assert.equal(manifest.version, "1.0.0");
  assert.ok(Number.isSafeInteger(manifest.builtAt) && manifest.builtAt > 0);
  assert.equal(Object.keys(manifest.files).length, 3);
  assert.match(manifest.files["pet.html"], /^[a-f0-9]{64}$/);
});

test("installs increasing versions without changing old UI or Rust files", (t) => {
  const { source, data, root } = fixture(t);
  const first = installWebcontent(source, data);
  const binary = join(root, "code-pet.exe");
  writeFileSync(binary, "unchanged Rust binary");
  writeFileSync(join(source, "assets/main.js"), "console.log('UI v2')");
  createManifest(source, "1.1.0");
  const second = installWebcontent(source, data);
  assert.equal(first.directoryVersion, "v1");
  assert.equal(second.directoryVersion, "v2");
  assert.equal(validateWebcontent(second.target).version, "1.1.0");
  assert.equal(validateWebcontent(first.target).version, "1.0.0");
  assert.equal(validateWebcontent(second.target).builtAt, validateWebcontent(source).builtAt);
  assert.equal(readFileSync(binary, "utf8"), "unchanged Rust binary");
});

test("requires semantic release versions and a build timestamp", (t) => {
  const { source } = fixture(t);
  for (const version of ['v1', '1.0', '01.0.0', '1.0.0-01']) assert.throws(() => createManifest(source, version), /semantic version/);
  for (const time of [0, -1, 1.5, Number.NaN]) assert.throws(() => createManifest(source, '1.0.0', time), /timestamp/);
  const manifest = createManifest(source, '2.0.0-beta.10+local', 1789228800000);
  assert.equal(validateWebcontent(source).builtAt, 1789228800000);
  delete manifest.builtAt;
  writeFileSync(join(source, 'manifest.json'), JSON.stringify(manifest));
  assert.throws(() => validateWebcontent(source), /timestamp/);
});

test("rejects corrupt source without installing a new version", (t) => {
  const { source, data } = fixture(t);
  const first = installWebcontent(source, data);
  writeFileSync(join(source, "assets/main.js"), "incomplete copy");
  assert.throws(() => installWebcontent(source, data), /checksum/);
  assert.equal(validateWebcontent(first.target).version, "1.0.0");
  assert.equal(existsSync(join(data, "webcontent", "v2")), false);
});

test("a newer complete version can replace a corrupt version without overwriting it", (t) => {
  const { source, data } = fixture(t);
  const first = installWebcontent(source, data);
  rmSync(join(first.target, "pet.html"));
  const second = installWebcontent(source, data);
  assert.equal(second.directoryVersion, "v2");
  assert.equal(validateWebcontent(second.target).version, "1.0.0");
  assert.equal(existsSync(join(first.target, "pet.html")), false);
});

test("uses numeric ordering and ignores staging directories", (t) => {
  const { source, data } = fixture(t);
  mkdirSync(join(data, "webcontent", "v9"), { recursive: true });
  mkdirSync(join(data, "webcontent", "v10"));
  mkdirSync(join(data, "webcontent", ".webcontent-stage-123"));
  const result = installWebcontent(source, data);
  assert.equal(result.directoryVersion, "v11");
});

test("rejects incompatible backend versions and missing entries", (t) => {
  const { source } = fixture(t);
  const path = join(source, "manifest.json");
  const manifest = JSON.parse(readFileSync(path, "utf8"));
  manifest.backendApiVersion += 1;
  writeFileSync(path, JSON.stringify(manifest));
  assert.throws(() => validateWebcontent(source), /Incompatible/);
  manifest.backendApiVersion -= 1;
  delete manifest.files["pet.html"];
  writeFileSync(path, JSON.stringify(manifest));
  assert.throws(() => validateWebcontent(source), /Missing entry/);
});

test("rejects path escapes, overlapping paths and App bundle destinations", (t) => {
  const { root, source, data } = fixture(t);
  assert.throws(() => installWebcontent(source, source), /overlap/);
  assert.throws(() => installWebcontent(source, join(root, "Code Pet.app", "Contents", "Resources")), /signed App bundle/);
  const path = join(source, "manifest.json");
  const manifest = JSON.parse(readFileSync(path, "utf8"));
  manifest.files["../secret.txt"] = "a".repeat(64);
  writeFileSync(path, JSON.stringify(manifest));
  assert.throws(() => validateWebcontent(source), /Invalid webcontent path/);
  assert.equal(existsSync(join(data, "webcontent")), false);
});

test("rejects destinations aliased to the source through a directory link", (t) => {
  const { root, source } = fixture(t);
  const alias = join(root, "alias");
  symlinkSync(source, alias, process.platform === "win32" ? "junction" : "dir");
  assert.throws(() => installWebcontent(source, alias), /overlap/);
  assert.equal(validateWebcontent(source).version, "1.0.0");
  assert.equal(existsSync(join(source, "webcontent")), false);
});

test("rejects App bundle aliases before creating directories", (t) => {
  const { root, source } = fixture(t);
  const bundle = join(root, "Code Pet.app");
  mkdirSync(bundle);
  const alias = join(root, "bundle-alias");
  symlinkSync(bundle, alias, process.platform === "win32" ? "junction" : "dir");
  assert.throws(() => installWebcontent(source, join(alias, "Contents", "Resources")), /signed App bundle/);
  assert.equal(existsSync(join(bundle, "Contents")), false);
});
