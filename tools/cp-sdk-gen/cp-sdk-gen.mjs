#!/usr/bin/env bun

import { createHash } from "node:crypto";
import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, relative, resolve } from "node:path";

import coreCargo from "../../sdk/rust/codepet-core-sdk/Cargo.toml" with { type: "text" };
import coreLib from "../../sdk/rust/codepet-core-sdk/src/lib.rs" with { type: "text" };
import agentCargo from "../../sdk/rust/codepet-agent-sdk/Cargo.toml" with { type: "text" };
import agentLib from "../../sdk/rust/codepet-agent-sdk/src/lib.rs" with { type: "text" };
import { providerRuntimeFiles } from "./provider-runtime.mjs";
import providerCargo from "../../sdk/rust/codepet-provider-sdk/Cargo.toml" with { type: "text" };
import gatewayRustHeartbeat from "../../sdk/rust/codepet-gateway-sdk/src/heartbeat.rs" with { type: "text" };
import gatewayDartHeartbeat from "../../sdk/dart/codepet-gateway-sdk/lib/src/heartbeat.dart" with { type: "text" };
import gatewayRustCargo from "../../sdk/rust/codepet-gateway-sdk/Cargo.toml" with { type: "text" };
import gatewayRustLib from "../../sdk/rust/codepet-gateway-sdk/src/lib.rs" with { type: "text" };
import lanRustCargo from "../../sdk/rust/codepet-lan-channel-sdk/Cargo.toml" with { type: "text" };
import lanRustLib from "../../sdk/rust/codepet-lan-channel-sdk/src/lib.rs" with { type: "text" };
import coreDartPubspec from "../../sdk/dart/codepet-core-sdk/pubspec.yaml" with { type: "text" };
import coreDartLib from "../../sdk/dart/codepet-core-sdk/lib/codepet_core_sdk.dart" with { type: "text" };
import agentDartPubspec from "../../sdk/dart/codepet-agent-sdk/pubspec.yaml" with { type: "text" };
import agentDartLib from "../../sdk/dart/codepet-agent-sdk/lib/codepet_agent_sdk.dart" with { type: "text" };
import gatewayDartPubspec from "../../sdk/dart/codepet-gateway-sdk/pubspec.yaml" with { type: "text" };
import gatewayDartLib from "../../sdk/dart/codepet-gateway-sdk/lib/codepet_gateway_sdk.dart" with { type: "text" };
import lanDartPubspec from "../../sdk/dart/codepet-lan-channel-sdk/pubspec.yaml" with { type: "text" };
import lanDartLib from "../../sdk/dart/codepet-lan-channel-sdk/lib/codepet_lan_channel_sdk.dart" with { type: "text" };
import guide from "./README.md" with { type: "text" };
import {
  GENERATOR_TARGET_INTERFACE,
  configureProtocolGenerator,
  generateProtocol,
} from "../protocol-codegen/generate.mjs";

const VERSION = "0.1.0";
const PACKAGE_IDS = new Set(["provider", "gateway", "lan-channel"]);

function fail(message) {
  throw new Error(message);
}

function valueAfter(arguments_, index, name) {
  const argument = arguments_[index];
  if (argument.startsWith(`${name}=`)) return [argument.slice(name.length + 1), index];
  const value = arguments_[index + 1];
  if (!value || value.startsWith("--")) fail(`${name} requires a value`);
  return [value, index + 1];
}

function parseArguments(arguments_) {
  const options = { language: undefined, output: undefined, protocol: undefined, package: "provider", role: undefined, check: false };
  for (let index = 0; index < arguments_.length; index += 1) {
    const argument = arguments_[index];
    if (argument === "--help" || argument === "-h") return { help: true };
    if (argument === "--check") {
      options.check = true;
      continue;
    }
    if (["--lang", "--output", "--protocol", "--package", "--role"].some((name) => argument === name || argument.startsWith(`${name}=`))) {
      const name = ["--lang", "--output", "--protocol", "--package", "--role"].find((candidate) => argument === candidate || argument.startsWith(`${candidate}=`));
      const [value, consumed] = valueAfter(arguments_, index, name);
      index = consumed;
      if (name === "--lang") options.language = value;
      if (name === "--output") options.output = value;
      if (name === "--protocol") options.protocol = value;
      if (name === "--package") options.package = value;
      if (name === "--role") options.role = value;
      continue;
    }
    fail(`unknown argument: ${argument}`);
  }
  if (!options.language) fail("--lang is required");
  if (!options.output) fail("--output is required");
  if (!PACKAGE_IDS.has(options.package)) fail(`--package must be provider, gateway, or lan-channel`);
  if (!["rust", "dart"].includes(options.language)) fail(`SDK target is not implemented for language: ${options.language}`);
  const defaultRole = options.package === "provider" ? "server" : options.package === "gateway" ? "client" : "models";
  options.role ??= defaultRole;
  const supportedRoles = options.package === "gateway" && options.language === "rust"
    ? new Set(["client", "server", "both"])
    : new Set([defaultRole]);
  if (!supportedRoles.has(options.role)) fail(`unsupported role ${options.role} for ${options.package}/${options.language}`);
  if (options.package === "provider" && options.language !== "rust") fail("Provider server SDK currently supports Rust only");
  return options;
}

async function exists(path) {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

async function defaultProtocolDirectory() {
  const adjacent = resolve(dirname(process.execPath), "protocol");
  if (await exists(adjacent)) return adjacent;
  const workingTree = resolve(process.cwd(), "protocol");
  if (await exists(workingTree)) return workingTree;
  fail("cannot find protocol resources; pass --protocol <protocol-dir>");
}

async function protocolLayout(protocolDirectory) {
  const distributedCore = resolve(protocolDirectory, "v1", "core");
  const distributedProvider = resolve(protocolDirectory, "v1", "schema");
  if (await exists(resolve(distributedCore, "schema.json")) && await exists(resolve(distributedProvider, "schema.json"))) {
    return { core: "v1/core", provider: "v1/schema" };
  }
  const sourceCore = resolve(protocolDirectory, "core", "v1");
  const sourceAgent = resolve(protocolDirectory, "agent", "v1");
  const sourceProvider = resolve(protocolDirectory, "provider", "v1");
  if (await exists(resolve(sourceCore, "schema.json")) && await exists(resolve(sourceAgent, "schema.json")) && await exists(resolve(sourceProvider, "schema.json"))) {
    return {
      core: "core/v1",
      agent: "agent/v1",
      provider: "provider/v1",
      gateway: "gateway/v1",
      "lan-channel": "channel/lan/v1",
    };
  }
  fail(`${protocolDirectory} does not contain protocol/v1/{core,schema} or source core/provider v1 resources`);
}

async function buildConfig(protocolDirectory, layout, options) {
  const coreSchema = JSON.parse(await readFile(resolve(protocolDirectory, layout.core, "schema.json"), "utf8"));
  const includeAgent = options.package === "provider" || options.package === "gateway";
  const agentSchema = includeAgent
    ? JSON.parse(await readFile(resolve(protocolDirectory, layout.agent, "schema.json"), "utf8"))
    : undefined;
  const selectedPath = layout[options.package];
  if (!selectedPath) fail(`protocol resources do not contain package: ${options.package}`);
  const selectedSchema = JSON.parse(await readFile(resolve(protocolDirectory, selectedPath, "schema.json"), "utf8"));
  const language = options.language;
  const packageMetadata = {
    provider: { id: "provider-v1", layer: "provider", version: 1, output: "codepet-provider-sdk/src/generated.rs" },
    gateway: {
      id: "gateway-v1",
      layer: "gateway",
      version: 1,
      output: language === "rust" ? "codepet-gateway-sdk/src/generated.rs" : "codepet-gateway-sdk/lib/src/generated.dart",
    },
    "lan-channel": {
      id: "channel-lan-v1",
      layer: "channel",
      version: 1,
      output: language === "rust" ? "codepet-lan-channel-sdk/src/generated.rs" : "codepet-lan-channel-sdk/lib/src/generated.dart",
    },
  }[options.package];
  const coreOutput = language === "rust"
    ? "codepet-core-sdk/src/generated.rs"
    : "codepet-core-sdk/lib/src/generated.dart";
  const agentOutput = language === "rust"
    ? "codepet-agent-sdk/src/generated.rs"
    : "codepet-agent-sdk/lib/src/generated.dart";
  return {
    generatorInterfaceVersion: 1,
    targets: [
      { id: "rust", interface: GENERATOR_TARGET_INTERFACE, status: "active" },
      { id: "typescript", interface: GENERATOR_TARGET_INTERFACE, status: "active" },
      { id: "dart", interface: GENERATOR_TARGET_INTERFACE, status: "active" },
      { id: "python", interface: GENERATOR_TARGET_INTERFACE, status: "planned" },
    ],
    packages: [
      {
        id: "core-v1",
        layer: "core",
        version: 1,
        schema: `${layout.core}/schema.json`,
        manifest: `${layout.core}/manifest.json`,
        dependencies: [],
        publicTypes: Object.keys(coreSchema.$defs ?? {}),
        outputs: { [language]: coreOutput },
      },
      ...(includeAgent ? [{
        id: "agent-v1",
        layer: "agent",
        version: 1,
        schema: `${layout.agent}/schema.json`,
        manifest: `${layout.agent}/manifest.json`,
        dependencies: ["core-v1"],
        publicTypes: Object.keys(agentSchema.$defs ?? {}),
        outputs: { [language]: agentOutput },
      }] : []),
      {
        id: packageMetadata.id,
        layer: packageMetadata.layer,
        version: packageMetadata.version,
        schema: `${selectedPath}/schema.json`,
        manifest: `${selectedPath}/manifest.json`,
        dependencies: includeAgent ? ["core-v1", "agent-v1"] : ["core-v1"],
        ...(options.package === "lan-channel" ? { publicTypes: Object.keys(selectedSchema.$defs ?? {}) } : {}),
        outputs: { [language]: packageMetadata.output },
      },
    ],
  };
}

async function digestProtocol(protocolDirectory, config) {
  const hash = createHash("sha256");
  for (const package_ of config.packages) {
    for (const key of ["schema", "manifest"]) {
      hash.update(package_[key]);
      hash.update("\0");
      hash.update(await readFile(resolve(protocolDirectory, package_[key])));
      hash.update("\0");
    }
  }
  return `sha256:${hash.digest("hex")}`;
}

async function updateFile(outputDirectory, path, content, check, stale) {
  const destination = resolve(outputDirectory, path);
  const current = await readFile(destination, "utf8").catch(() => undefined);
  if (current === content) return;
  if (check) {
    stale.push(path);
    return;
  }
  await mkdir(dirname(destination), { recursive: true });
  await writeFile(destination, content, "utf8");
}

export async function runCpSdkGen(arguments_, currentDirectory = process.cwd()) {
  const options = parseArguments(arguments_);
  if (options.help) {
    return "Usage: cp-sdk-gen --package provider|gateway|lan-channel --role client|server|both|models --lang rust|dart --output <sdk-dir> [--protocol <protocol-dir>] [--check]";
  }
  const outputDirectory = resolve(currentDirectory, options.output);
  const protocolDirectory = options.protocol
    ? resolve(currentDirectory, options.protocol)
    : await defaultProtocolDirectory();
  const layout = await protocolLayout(protocolDirectory);
  const config = await buildConfig(protocolDirectory, layout, options);
  configureProtocolGenerator({
    sourceRoot: dirname(protocolDirectory),
    protocolDirectory,
    generatedOutputRoot: outputDirectory,
  });
  await generateProtocol({ checkMode: options.check, targets: [options.language], config, role: options.role });

  const lock = `${JSON.stringify({
    generator: "cp-sdk-gen",
    generatorVersion: VERSION,
    package: options.package,
    role: options.role,
    protocolVersion: config.packages.find((package_) => package_.id === `${options.package === "lan-channel" ? "channel-lan" : options.package}-v1`).version,
    protocolDigest: await digestProtocol(protocolDirectory, config),
    language: options.language,
  }, null, 2)}\n`;
  const staticFiles = new Map([
    ["README.md", guide],
    ["cp-sdk-gen.lock.json", lock],
  ]);
  if (options.language === "rust") {
    const packageDirectory = options.package === "provider"
      ? "codepet-provider-sdk"
      : options.package === "gateway"
        ? "codepet-gateway-sdk"
        : "codepet-lan-channel-sdk";
    const members = ["codepet-core-sdk", ...(options.package === "lan-channel" ? [] : ["codepet-agent-sdk"]), packageDirectory];
    staticFiles.set("Cargo.toml", `[workspace]\nmembers = [\n${members.map((member) => `    "${member}",`).join("\n")}\n]\nresolver = "2"\n`);
    staticFiles.set("codepet-core-sdk/Cargo.toml", coreCargo);
    staticFiles.set("codepet-core-sdk/src/lib.rs", coreLib);
    if (options.package !== "lan-channel") {
      staticFiles.set("codepet-agent-sdk/Cargo.toml", agentCargo);
      staticFiles.set("codepet-agent-sdk/src/lib.rs", agentLib);
    }
    if (options.package === "provider") {
      staticFiles.set("codepet-provider-sdk/Cargo.toml", providerCargo);
      for (const [relativePath, source] of providerRuntimeFiles) {
        staticFiles.set(`codepet-provider-sdk/src/${relativePath}`, source);
      }
    } else if (options.package === "gateway") {
      staticFiles.set("codepet-gateway-sdk/Cargo.toml", gatewayRustCargo);
      staticFiles.set("codepet-gateway-sdk/src/lib.rs", gatewayRustLib);
      staticFiles.set("codepet-gateway-sdk/src/heartbeat.rs", gatewayRustHeartbeat);
    } else {
      staticFiles.set("codepet-lan-channel-sdk/Cargo.toml", lanRustCargo);
      staticFiles.set("codepet-lan-channel-sdk/src/lib.rs", lanRustLib);
    }
  } else {
    staticFiles.set("codepet-core-sdk/pubspec.yaml", coreDartPubspec);
    staticFiles.set("codepet-core-sdk/lib/codepet_core_sdk.dart", coreDartLib);
    if (options.package === "gateway") {
      staticFiles.set("codepet-agent-sdk/pubspec.yaml", agentDartPubspec);
      staticFiles.set("codepet-agent-sdk/lib/codepet_agent_sdk.dart", agentDartLib);
      staticFiles.set("codepet-gateway-sdk/pubspec.yaml", gatewayDartPubspec);
      staticFiles.set("codepet-gateway-sdk/lib/codepet_gateway_sdk.dart", gatewayDartLib);
      staticFiles.set("codepet-gateway-sdk/lib/src/heartbeat.dart", gatewayDartHeartbeat);
    } else {
      staticFiles.set("codepet-lan-channel-sdk/pubspec.yaml", lanDartPubspec);
      staticFiles.set("codepet-lan-channel-sdk/lib/codepet_lan_channel_sdk.dart", lanDartLib);
    }
  }
  const stale = [];
  for (const [path, content] of staticFiles) {
    await updateFile(outputDirectory, path, content, options.check, stale);
  }
  if (stale.length > 0) fail(`generated SDK is stale:\n${stale.map((path) => `- ${path}`).join("\n")}`);
  return options.check
    ? `${options.package} ${options.role} SDK is up to date in ${outputDirectory}`
    : `Generated ${options.package} ${options.role} SDK from ${relative(currentDirectory, protocolDirectory) || "."} in ${outputDirectory}`;
}

if (import.meta.main) {
  runCpSdkGen(process.argv.slice(2)).then(console.log).catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
