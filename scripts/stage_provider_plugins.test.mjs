import assert from "node:assert/strict";
import { cp, chmod, mkdir, mkdtemp, readFile, readdir, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  PROVIDERS,
  SDK_GENERATOR,
  providerCargoBuildArguments,
  resolveBunExecutable,
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
  assert.deepEqual(bun.slice(0, 2), ["build", path.join("/repository", "tools", "cp-sdk-gen", "cp-sdk-gen.mjs")]);
  assert.equal(bun.includes("bun-darwin-arm64"), true);
});

test("Windows Bun discovery resolves npm shims to the native executable", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "codepet-bun 中文-"));
  const native = path.join(root, "node_modules", "bun", "bin", "bun.exe");
  await mkdir(path.dirname(native), { recursive: true });
  await writeFile(path.join(root, "bun.cmd"), "shim must not be executed");
  await writeFile(native, "native fixture");
  assert.equal(await resolveBunExecutable({ env: { Path: root }, platform: "win32" }), native);
  const standalone = path.join(root, "bun.exe");
  await writeFile(standalone, "standalone fixture");
  assert.equal(await resolveBunExecutable({ env: { PATH: root }, platform: "win32" }), standalone);
  assert.equal(await resolveBunExecutable({ env: { BUN: native, PATH: root }, platform: "win32" }), native);
  assert.equal(await resolveBunExecutable({ env: { PATH: root }, platform: "darwin" }), "bun");
});

test("staging contains Provider plugins and only the language-neutral extension contract", async () => {
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
  await mkdir(path.join(root, "protocol", "agent", "v1"), { recursive: true });
  await mkdir(path.join(root, "protocol", "provider", "v1", "fixtures"), { recursive: true });
  await mkdir(path.join(root, "protocol", "gateway", "v1"), { recursive: true });
  await mkdir(path.join(root, "protocol", "channel", "lan", "v1"), { recursive: true });
  for (const [layer, version] of [["core", 1], ["agent", 1], ["provider", 1], ["gateway", 1]]) {
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

  await cp(path.resolve(import.meta.dirname,"../protocol/v1"),path.join(root,"protocol/v1"),{recursive:true});
  for (const [target, windowsTarget] of [
    [undefined, process.platform === "win32"],
    ["x86_64-unknown-linux-gnu", false],
    ["x86_64-pc-windows-msvc", true],
  ]) {
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
      const executable = `${provider.packageName}${windowsTarget ? ".exe" : ""}`;
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
      if (!windowsTarget && process.platform !== "win32") {
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
    assert.deepEqual((await readdir(sdkStagingDirectory)).sort(),["v1"]);
    const sdkRoot=path.join(sdkStagingDirectory,"v1");
    assert.deepEqual((await readdir(sdkRoot)).sort(),["README.md","protocol","sdk-manifest.json"]);
    const index=JSON.parse(await readFile(path.join(sdkRoot,"sdk-manifest.json"),"utf8"));
    assert.equal(index.package,"provider"); assert.equal(index.sdkVersion,1);
    assert.equal(index.generator,undefined);
    assert.equal(index.methods,"protocol/sdk/provider/methods.json");
    assert.deepEqual((await readdir(path.join(sdkRoot,"protocol"))).sort(),["manifest.schema.json","sdk"]);
    for(const entry of [index.commonSchema,index.agentDomainSchema,index.providerLifecycleSchema,index.providerMessagesSchema])assert.ok((await stat(path.join(sdkRoot,entry))).isFile());

  }
});
