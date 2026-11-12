import assert from "node:assert/strict";
import { cp, mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { runCpSdkGen } from "./cp-sdk-gen.mjs";
import { providerRuntimeFiles } from "./provider-runtime.mjs";
import { transportRuntimeFiles } from "./transport-runtime.mjs";

const repositoryRoot = path.resolve(import.meta.dirname, "../..");

test("cp-sdk-gen compiles protocol input into a complete Rust SDK", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "cp-sdk-gen-js-"));
  const output = path.join(root, "sdk");
  await runCpSdkGen([
    "--lang", "rust",
    "--protocol", path.join(repositoryRoot, "protocol"),
    "--output", output,
  ], root);
  const generated = await readFile(path.join(output, "codepet-provider-sdk", "src", "generated.rs"), "utf8");
  const agentGenerated = await readFile(path.join(output, "codepet-agent-sdk", "src", "generated.rs"), "utf8");
  assert.match(generated, /pub trait ProtocolServer/);
  assert.match(generated, /pub use codepet_agent_sdk::\*;/);
  assert.match(agentGenerated, /pub enum ConversationItem/);
  assert.match(await readFile(path.join(output,"codepet-core-sdk/src/generated.rs"),"utf8"), /pub struct ProviderTransportHello/);
  assert.match(await readFile(path.join(output, "codepet-transport", "src", "protocol.rs"), "utf8"), /stdio-codepet-mux-v1/);
  assert.match(await readFile(path.join(output, "codepet-provider-sdk", "src", "runtime", "mux.rs"), "utf8"), /serve_mux_with_io/);
  // Every handwritten module (including local unit tests) must survive standalone export.
  const runtimeRoot = path.join(repositoryRoot, "sdk", "rust", "codepet-provider-sdk", "src");
  const sourcePaths = (await readdir(runtimeRoot, { recursive: true }))
    .filter((name) => name.endsWith(".rs") && name !== "generated.rs")
    .map((name) => name.split(path.sep).join("/"));
  assert.deepEqual([...providerRuntimeFiles.keys()].sort(), sourcePaths.sort());
  for (const relativePath of sourcePaths) {
    assert.equal(
      await readFile(path.join(output, "codepet-provider-sdk", "src", relativePath), "utf8"),
      await readFile(path.join(runtimeRoot, relativePath), "utf8"),
      `Standalone SDK omitted or changed ${relativePath}`,
    );
  }
  const transportRoot=path.join(repositoryRoot,"sdk/rust/codepet-transport/src");
  const transportPaths=(await readdir(transportRoot,{recursive:true})).filter(name=>name.endsWith(".rs")).map(name=>name.split(path.sep).join("/"));
  assert.deepEqual([...transportRuntimeFiles.keys()].sort(),transportPaths.sort());
  for(const relativePath of transportPaths) assert.equal(await readFile(path.join(output,"codepet-transport/src",relativePath),"utf8"),await readFile(path.join(transportRoot,relativePath),"utf8"));
  assert.match(generated, /ProviderInitializeRequest/);
  assert.doesNotMatch(generated, /pub struct ProtocolClient/);
  assert.match(
    await readFile(path.join(output, "codepet-transport", "src", "protocol.rs"), "utf8"),
    /FRAME_MAGIC: \[u8; 4\] = \*b"CPRF"/,
  );
  assert.match(
    await readFile(path.join(output, "codepet-provider-sdk", "Cargo.toml"), "utf8"),
    /zstd = "0\.13"/,
  );
  assert.equal(
    (await readFile(path.join(output, "codepet-core-sdk", "src", "generated.rs"), "utf8")).replaceAll("\r\n", "\n"),
    (await readFile(path.join(repositoryRoot, "sdk", "rust", "codepet-core-sdk", "src", "generated.rs"), "utf8")).replaceAll("\r\n", "\n"),
  );
  assert.match(await readFile(path.join(output, "README.md"), "utf8"), /安装并让 Code Pet 发现/);
  await runCpSdkGen([
    "--lang=rust",
    `--protocol=${path.join(repositoryRoot, "protocol")}`,
    `--output=${output}`,
    "--check",
  ], root);
});

test("cp-sdk-gen fails closed for an unavailable Provider target", async () => {
  await assert.rejects(
    runCpSdkGen(["--lang", "dart", "--output", "sdk"], repositoryRoot),
    /supports Rust only/,
  );
});

test("cp-sdk-gen emits Gateway client and server packages from the same schema", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "cp-sdk-gen-gateway-"));
  const dartOutput = path.join(root, "dart");
  const rustOutput = path.join(root, "rust");
  await runCpSdkGen([
    "--package", "gateway",
    "--role", "client",
    "--lang", "dart",
    "--protocol", path.join(repositoryRoot, "protocol"),
    "--output", dartOutput,
  ], root);
  await runCpSdkGen([
    "--package", "gateway",
    "--role", "server",
    "--lang", "rust",
    "--protocol", path.join(repositoryRoot, "protocol"),
    "--output", rustOutput,
  ], root);
  const dartGenerated = await readFile(path.join(dartOutput, "codepet-gateway-sdk", "lib", "src", "generated.dart"), "utf8");
  const dartAgentGenerated = await readFile(path.join(dartOutput, "codepet-agent-sdk", "lib", "src", "generated.dart"), "utf8");
  const rustGenerated = await readFile(path.join(rustOutput, "codepet-gateway-sdk", "src", "generated.rs"), "utf8");
  assert.match(
    dartGenerated,
    /final class ProtocolClient/,
  );
  assert.match(
    rustGenerated,
    /pub trait ProtocolServer/,
  );
  assert.match(dartGenerated, /import 'package:codepet_agent_sdk\/codepet_agent_sdk\.dart';/);
  assert.match(dartAgentGenerated, /sealed class ConversationItem/);
  assert.doesNotMatch(rustGenerated, /pub struct ProtocolClient/);
});


test("SDK export reads functional IDL and hashes its policy instead of trusting compatibility files", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "cp-sdk-modular-"));
  const protocol = path.join(root, "protocol"), output = path.join(root, "sdk");
  await cp(path.join(repositoryRoot, "protocol"), protocol, { recursive: true });
  for (const file of ["core/v1/schema.json", "agent/v1/schema.json", "gateway/v1/schema.json", "gateway/v1/manifest.json"]) {
    await writeFile(path.join(protocol, file), "not an IDL input");
  }
  const args = ["--package", "gateway", "--lang", "rust", "--protocol", protocol, "--output", output];
  await runCpSdkGen(args, root);
  const before = JSON.parse(await readFile(path.join(output, "cp-sdk-gen.lock.json")));
  const policyFile = path.join(protocol, "v1/gateway/session/messages.schema.json");
  const schema = JSON.parse(await readFile(policyFile));
  schema["x-codepet-lifecycle"].heartbeatIntervalMs = 15000;
  await writeFile(policyFile, JSON.stringify(schema));
  await runCpSdkGen(args, root);
  const after = JSON.parse(await readFile(path.join(output, "cp-sdk-gen.lock.json")));
  assert.notEqual(before.protocolDigest, after.protocolDigest);
  assert.match(await readFile(path.join(output, "codepet-gateway-sdk/src/generated.rs"), "utf8"), /GATEWAY_HEARTBEAT_INTERVAL_MS: u64 = 15000/);
});


test('channel-control exports standalone typed models without unrelated domain packages', async () => {
  const output = await mkdtemp(path.join(os.tmpdir(), 'cp-sdk-control-'));
  const args = ['--package', 'channel-control', '--lang', 'dart', '--protocol', path.join(repositoryRoot, 'protocol'), '--output', output];
  await runCpSdkGen(args);
  const generated = await readFile(path.join(output, 'codepet-channel-control-sdk/lib/src/generated.dart'), 'utf8');
  assert.match(generated, /final class ControlPrepare/);
  assert.match(generated, /final class ControlActivate/);
  assert.ok(!(await readdir(output)).includes('codepet-core-sdk'));
  await runCpSdkGen([...args, '--check']);
  await assert.rejects(runCpSdkGen(['--package', 'channel-control', '--lang', 'rust', '--output', output]), /Dart only/);
});

test('Remote SDK sync updates and checks all contracts with one source digest', async () => {
  const { syncRemoteSdk } = await import('../../scripts/sync_remote_sdk.mjs');
  const remote = await mkdtemp(path.join(os.tmpdir(), 'cp-remote-sync-'));
  await writeFile(path.join(remote, 'pubspec.yaml'), 'name: codepet_remote\n');
  await syncRemoteSdk(remote);
  const digests = [];
  for (const part of ['gateway', 'lan', 'channel-control']) {
    digests.push(JSON.parse(await readFile(path.join(remote, 'sdk', part, 'cp-sdk-gen.lock.json'))).protocolDigest);
  }
  assert.equal(new Set(digests).size, 1);
  await syncRemoteSdk(remote, { check: true });
  await writeFile(path.join(remote, 'sdk/channel-control/codepet-channel-control-sdk/lib/src/generated.dart'), 'stale');
  await assert.rejects(syncRemoteSdk(remote, { check: true }), /stale/);
  await writeFile(path.join(remote, 'pubspec.yaml'), 'name: unrelated_app\n');
  await assert.rejects(syncRemoteSdk(remote), /CodePet Remote/);
});
