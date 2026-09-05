import assert from "node:assert/strict";
import { mkdtemp, readFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { runCpSdkGen } from "./cp-sdk-gen.mjs";

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
  assert.match(generated, /ProviderInitializeRequest/);
  assert.doesNotMatch(generated, /pub struct ProtocolClient/);
  assert.equal(
    await readFile(path.join(output, "codepet-core-sdk", "src", "generated.rs"), "utf8"),
    await readFile(path.join(repositoryRoot, "sdk", "rust", "codepet-core-sdk", "src", "generated.rs"), "utf8"),
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
