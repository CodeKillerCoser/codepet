import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, readFile, readdir, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { PROVIDERS, stageProviderPlugins } from "./stage_provider_plugins.mjs";

test("staging contains the three Provider manifests and platform executables", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "codepet-provider-stage-"));
  const binaries = new Map();
  for (const provider of PROVIDERS) {
    const sourceDirectory = path.join(root, "crates", "providers", provider.packageName);
    const binary = path.join(root, "build", provider.packageName);
    await mkdir(sourceDirectory, { recursive: true });
    await mkdir(path.dirname(binary), { recursive: true });
    await writeFile(
      path.join(sourceDirectory, "codepet-provider.json"),
      JSON.stringify({
        manifestVersion: 1,
        pluginId: provider.pluginId,
        displayName: provider.name,
        executable: provider.packageName,
        enabled: true,
        instances: [],
      }),
    );
    await writeFile(binary, provider.name);
    await chmod(binary, 0o600);
    binaries.set(provider.name, binary);
  }

  for (const target of [undefined, "x86_64-pc-windows-msvc"]) {
    const stagingDirectory = path.join(root, target || "native");
    await stageProviderPlugins({
      repositoryRoot: root,
      stagingDirectory,
      target,
      binaries,
    });
    assert.deepEqual(
      (await readdir(stagingDirectory)).sort(),
      [".gitkeep", "claude", "codex", "opencode"],
    );
    for (const provider of PROVIDERS) {
      const providerDirectory = path.join(stagingDirectory, provider.name);
      const executable = `${provider.packageName}${target ? ".exe" : ""}`;
      const manifest = JSON.parse(
        await readFile(path.join(providerDirectory, "codepet-provider.json"), "utf8"),
      );
      assert.equal(manifest.pluginId, provider.pluginId);
      assert.equal(manifest.executable, executable);
      assert.deepEqual((await readdir(providerDirectory)).sort(), [
        "codepet-provider.json",
        executable,
      ].sort());
      if (!target) {
        assert.notEqual((await stat(path.join(providerDirectory, executable))).mode & 0o111, 0);
      }
    }
  }
});
