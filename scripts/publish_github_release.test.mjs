import { test } from "node:test";
import assert from "node:assert/strict";
import { workflowArguments } from "./publish_github_release.mjs";

test("build dispatch only sends build inputs", () => {
  assert.deepEqual(workflowArguments("build", ["--platform", "mac", "--version", "0.3.11-beta"]), ["workflow", "run", "build.yml", "--ref", "main", "--raw-field", "platform=mac", "--raw-field", "version=0.3.11-beta"]);
});
test("release dispatch pins a build run and preserves literal release notes", () => {
  const notes = 'First line\n`$(echo literal)` and @file';
  const result = workflowArguments("release", ["--build-run-id", "12345", "--release-notes", notes]);
  assert.deepEqual(result, ["workflow", "run", "release.yml", "--ref", "main", "--raw-field", "build_run_id=12345", "--raw-field", `release_notes=${notes}`]);
});
test("release rejects old rebuild options and requires an explicit run", () => {
  assert.throws(() => workflowArguments("release", []), /build-run-id/);
  assert.throws(() => workflowArguments("release", ["--version", "0.3.11"]), /Unknown option/);
  assert.throws(() => workflowArguments("release", ["--build-run-id", "../runs"]), /build-run-id/);
  assert.throws(() => workflowArguments("build", ["--platform", "linux"]), /platform/);
});
