import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, readFile, readdir, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  PROVIDERS,
  SDK_GENERATOR,
  providerCargoBuildArguments,
  sdkGeneratorBuildArguments,
  stageProviderPlugins,
  stageProviderSdkResources,
} from "./stage_provider_plugins.mjs";

test("Provider build profile selects the matching Cargo output profile", () => {
  const release = providerCargoBuildArguments("/repository", undefined, "release");
  const debug = providerCargoBuildArguments("/repository", undefined, "debug");
  assert.equal(release.includes("--release"), true);
  assert.equal(debug.includes("--release"), false);
  for (const provider of PROVIDERS) {
    assert.equal(release.includes(provider.packageName), true);
    assert.equal(debug.includes(provider.packageName), true);
  }
  assert.equal(release.includes(SDK_GENERATOR.packageName), false);
  assert.equal(debug.includes(SDK_GENERATOR.packageName), false);
  const bun = sdkGeneratorBuildArguments("/repository", "/output/cp-sdk-gen", "aarch64-apple-darwin");
  assert.deepEqual(bun.slice(0, 2), ["build", "/repository/tools/cp-sdk-gen/cp-sdk-gen.mjs"]);
  assert.equal(bun.includes("bun-darwin-arm64"), true);
});

test("staging contains Provider plugins, JSON-RPC resources, and cp-sdk-gen", async () => {
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
        icon: provider.icon,
        executable: provider.packageName,
        enabled: true,
        instances: [],
      }),
    );
    await writeFile(binary, provider.name);
    await chmod(binary, 0o600);
    binaries.set(provider.name, binary);
  }
  const generatorBinary = path.join(root, "build", SDK_GENERATOR.packageName);
  await writeFile(generatorBinary, "cp-sdk-gen");
  await chmod(generatorBinary, 0o600);
  binaries.set(SDK_GENERATOR.name, generatorBinary);

  await mkdir(path.join(root, "tools", "cp-sdk-gen"), { recursive: true });
  await writeFile(
    path.join(root, "tools", "cp-sdk-gen", "README.md"),
    "# Provider SDK\n\n## cp-sdk-gen 是什么\n\n## 安装并让 Code Pet 发现\n",
  );
  await mkdir(path.join(root, "protocol", "core", "v1"), { recursive: true });
  await mkdir(path.join(root, "protocol", "provider", "v1", "fixtures"), { recursive: true });
  await mkdir(path.join(root, "protocol", "gateway", "v1"), { recursive: true });
  await mkdir(path.join(root, "protocol", "channel", "lan", "v1"), { recursive: true });
  for (const [layer, version] of [["core", 1], ["provider", 1], ["gateway", 1]]) {
    await writeFile(
      path.join(root, "protocol", layer, `v${version}`, "schema.json"),
      JSON.stringify({
        $id: `codepet.${layer}.v1`,
        ...(layer === "provider"
          ? { dependency: { $ref: "../../core/v1/schema.json#/$defs/RequestId" } }
          : {}),
      }),
    );
    await writeFile(
      path.join(root, "protocol", layer, `v${version}`, "manifest.json"),
      JSON.stringify({
        name: `codepet-${layer}`,
        version,
        ...(layer === "provider"
          ? { dependency: { $ref: "../../core/v1/schema.json#/$defs/RpcError" } }
          : {}),
      }),
    );
  }
  for (const file of ["schema.json", "manifest.json"]) {
    await writeFile(path.join(root, "protocol", "channel", "lan", "v1", file), "{}");
  }
  await writeFile(
    path.join(root, "protocol", "provider", "v1", "fixtures", "initialize.json"),
    "{}",
  );

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
      ["claude", "codex", "opencode"],
    );
    for (const provider of PROVIDERS) {
      const providerDirectory = path.join(stagingDirectory, provider.name);
      const executable = `${provider.packageName}${target ? ".exe" : ""}`;
      const manifest = JSON.parse(
        await readFile(path.join(providerDirectory, "codepet-provider.json"), "utf8"),
      );
      assert.equal(manifest.pluginId, provider.pluginId);
      assert.equal(manifest.icon, provider.icon);
      assert.equal(manifest.executable, executable);
      assert.deepEqual((await readdir(providerDirectory)).sort(), [
        "codepet-provider.json",
        executable,
      ].sort());
      if (!target) {
        assert.notEqual((await stat(path.join(providerDirectory, executable))).mode & 0o111, 0);
      }
    }

    const sdkStagingDirectory = path.join(root, `${target || "native"}-sdk`);
    await stageProviderSdkResources({
      repositoryRoot: root,
      stagingDirectory: sdkStagingDirectory,
      target,
      binaries,
    });
    const generatorExecutable = `cp-sdk-gen${target ? ".exe" : ""}`;
    assert.equal((await stat(path.join(sdkStagingDirectory, generatorExecutable))).isFile(), true);
    if (!target) {
      assert.notEqual((await stat(path.join(sdkStagingDirectory, generatorExecutable))).mode & 0o111, 0);
    }
    const index = JSON.parse(
      await readFile(path.join(sdkStagingDirectory, "codepet-provider-sdk.json"), "utf8"),
    );
    assert.equal(index.protocol.name, "codepet-provider");
    assert.equal(index.protocol.version, 1);
    assert.equal(index.protocol.schema, "protocol/provider/v1/schema.json");
    assert.equal(index.protocol.manifest, "protocol/provider/v1/manifest.json");
    assert.equal(index.protocol.fixtures, "protocol/provider/v1/fixtures/index.json");
    assert.deepEqual(index.protocol.dependencies, [
      "protocol/core/v1/schema.json",
      "protocol/core/v1/manifest.json",
    ]);
    assert.equal(index.generator.executable, generatorExecutable);
    assert.equal(index.generator.implementation, "javascript-bun-compile");
    assert.equal(index.generator.protocolRoot, "protocol");
    assert.deepEqual(index.generator.languages, ["rust"]);
    const guide = await readFile(path.join(sdkStagingDirectory, "README.md"), "utf8");
    assert.equal(guide.includes("cp-sdk-gen 是什么"), true);
    assert.equal(guide.includes("安装并让 Code Pet 发现"), true);
    const providerSchemaPath = path.join(
      sdkStagingDirectory,
      "protocol",
      "provider",
      "v1",
      "schema.json",
    );
    assert.equal(
      (await stat(providerSchemaPath)).isFile(),
      true,
    );
    assert.equal(
      (await stat(path.join(sdkStagingDirectory, "protocol", "provider", "v1", "fixtures", "initialize.json"))).isFile(),
      true,
    );
    assert.equal(
      (await stat(path.join(sdkStagingDirectory, "protocol", "core", "v1", "schema.json"))).isFile(),
      true,
    );
    assert.equal(
      (await readFile(providerSchemaPath, "utf8")).includes("../../core/v1/schema.json"),
      true,
    );
    const providerManifest = await readFile(
      path.join(sdkStagingDirectory, "protocol", "provider", "v1", "manifest.json"),
      "utf8",
    );
    assert.equal(providerManifest.includes("../../core/v1/schema.json"), true);
    const sdkIndex = JSON.parse(await readFile(path.join(sdkStagingDirectory, "codepet-sdk.json"), "utf8"));
    assert.deepEqual(sdkIndex.packages.map((item) => item.package), ["provider", "gateway", "lan-channel"]);
  }
});
