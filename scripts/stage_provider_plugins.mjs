#!/usr/bin/env node
import { exportProviderSdk } from "./export_provider_sdk.mjs";

import { chmod, copyFile, cp, mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

export const PROVIDERS = Object.freeze([
  {
    name: "codex",
    packageName: "codepet-provider-codex",
    pluginId: "dev.codepet.codex",
    icon: "https://avatars.githubusercontent.com/u/14957082?s=200&v=4",
  },
  {
    name: "opencode",
    packageName: "codepet-provider-opencode",
    pluginId: "dev.codepet.opencode",
    icon: "https://opencode.ai/favicon-96x96-v3.png",
  },
  {
    name: "claude",
    packageName: "codepet-provider-claude",
    pluginId: "dev.codepet.claude",
    icon: "https://cdn.prod.website-files.com/6889473510b50328dbb70ae6/68c33859cc6cd903686c66a2_apple-touch-icon.png",
  },
]);

export const SDK_GENERATOR = Object.freeze({
  name: "cp-sdk-gen",
  packageName: "cp-sdk-gen",
});

const SCRIPT_DIRECTORY = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_REPOSITORY_ROOT = path.resolve(SCRIPT_DIRECTORY, "..");
const UNIVERSAL_MACOS_TARGET = "universal-apple-darwin";

function parseArguments(argv) {
  const options = { profile: "release" };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--target") {
      options.target = argv[index + 1];
      index += 1;
    } else if (argument === "--staging-dir") {
      options.stagingDirectory = argv[index + 1];
      index += 1;
    } else if (argument === "--profile") {
      options.profile = argv[index + 1];
      index += 1;
    } else {
      throw new Error(`unknown argument: ${argument}`);
    }
  }
  if (!new Set(["debug", "release"]).has(options.profile)) {
    throw new Error(`--profile must be debug or release, received: ${options.profile}`);
  }
  return options;
}

function isWindowsTarget(target) {
  return target?.includes("windows") ?? process.platform === "win32";
}

function binaryFileName(provider, target) {
  return `${provider.packageName}${isWindowsTarget(target) ? ".exe" : ""}`;
}

function run(command, args, repositoryRoot) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    env: process.env,
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status}`);
  }
}

export function providerCargoBuildArguments(repositoryRoot, target, profile = "release") {
  const args = [
    "build",
    "--manifest-path",
    path.join(repositoryRoot, "crates", "Cargo.toml"),
  ];
  if (profile === "release") {
    args.push("--release");
  }
  if (target) {
    args.push("--target", target);
  }
  for (const provider of PROVIDERS) {
    args.push("--package", provider.packageName, "--bin", provider.packageName);
  }
  return args;
}

function cargoBuild(repositoryRoot, target, profile) {
  const args = providerCargoBuildArguments(repositoryRoot, target, profile);
  run(process.env.CARGO || "cargo", args, repositoryRoot);
}

function bunTarget(target) {
  if (!target) return undefined;
  const targets = new Map([
    ["aarch64-apple-darwin", "bun-darwin-arm64"],
    ["x86_64-apple-darwin", "bun-darwin-x64"],
    ["x86_64-pc-windows-msvc", "bun-windows-x64"],
    ["x86_64-unknown-linux-gnu", "bun-linux-x64"],
    ["aarch64-unknown-linux-gnu", "bun-linux-arm64"],
  ]);
  const resolved = targets.get(target);
  if (!resolved) {
    throw new Error(`cp-sdk-gen Bun compile target is not mapped: ${target}`);
  }
  return resolved;
}

export function sdkGeneratorBuildArguments(repositoryRoot, output, target, profile = "release") {
  const args = [
    "build",
    path.join(repositoryRoot, "tools", "cp-sdk-gen", "cp-sdk-gen.mjs"),
    "--compile",
    "--outfile",
    output,
  ];
  const compileTarget = bunTarget(target);
  if (compileTarget) args.push("--target", compileTarget);
  if (profile === "release") args.push("--minify");
  return args;
}

export async function resolveBunExecutable({ env = process.env, platform = process.platform } = {}) {
  if (env.BUN) return env.BUN;
  if (platform !== "win32") return "bun";
  const searchPath = Object.entries(env).find(([key]) => key.toLowerCase() === "path")?.[1] ?? "";
  for (const directory of searchPath.split(path.delimiter).filter(Boolean)) {
    if (!path.isAbsolute(directory)) continue;
    // npm's .cmd/.ps1 shims cannot be passed to spawnSync without a shell.
    for (const executable of [
      path.join(directory, "bun.exe"),
      path.join(directory, "node_modules", "bun", "bin", "bun.exe"),
    ]) {
      if (await stat(executable).then((entry) => entry.isFile(), () => false)) return executable;
    }
  }
  return "bun";
}

function builtBinaryPath(repositoryRoot, provider, target, profile) {
  const targetRoot = path.resolve(repositoryRoot, process.env.CARGO_TARGET_DIR || "crates/target");
  const targetDirectory = target ? path.join(targetRoot, target, profile) : path.join(targetRoot, profile);
  return path.join(targetDirectory, binaryFileName(provider, target));
}

async function assertFile(filePath, description) {
  const metadata = await stat(filePath).catch(() => null);
  if (!metadata?.isFile()) {
    throw new Error(`${description} is missing: ${filePath}`);
  }
}

async function buildProviderBinaries(repositoryRoot, target, profile) {
  if (target === UNIVERSAL_MACOS_TARGET) {
    if (process.platform !== "darwin") {
      throw new Error("universal macOS Provider binaries can only be built on macOS");
    }
    const architectureTargets = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
    for (const architectureTarget of architectureTargets) {
      cargoBuild(repositoryRoot, architectureTarget, profile);
    }
    const universalDirectory = path.join(
      path.resolve(repositoryRoot, process.env.CARGO_TARGET_DIR || "crates/target"),
      UNIVERSAL_MACOS_TARGET,
      profile,
    );
    await mkdir(universalDirectory, { recursive: true });
    const binaries = new Map();
    for (const artifact of PROVIDERS) {
      const inputs = architectureTargets.map((architectureTarget) =>
        builtBinaryPath(repositoryRoot, artifact, architectureTarget, profile),
      );
      for (const input of inputs) {
        await assertFile(input, `${artifact.name} architecture binary`);
      }
      const output = path.join(universalDirectory, binaryFileName(artifact, target));
      run("lipo", ["-create", ...inputs, "-output", output], repositoryRoot);
      binaries.set(artifact.name, output);
    }
    return binaries;
  }

  cargoBuild(repositoryRoot, target, profile);
  const binaries = new Map(
    PROVIDERS.map((artifact) => [
      artifact.name,
      builtBinaryPath(repositoryRoot, artifact, target, profile),
    ]),
  );
  return binaries;
}

export async function stageProviderPlugins({
  repositoryRoot = DEFAULT_REPOSITORY_ROOT,
  stagingDirectory = path.join(repositoryRoot, "src-tauri", "resources", "provider-plugins"),
  target,
  binaries,
}) {
  const resolvedRepositoryRoot = path.resolve(repositoryRoot);
  const resolvedStagingDirectory = path.resolve(stagingDirectory);
  await mkdir(resolvedStagingDirectory, { recursive: true });
  for (const entry of await readdir(resolvedStagingDirectory)) {
    if (entry === ".gitkeep") continue;
    await rm(path.join(resolvedStagingDirectory, entry), { recursive: true, force: true });
  }

  for (const provider of PROVIDERS) {
    const sourceManifest = path.join(
      resolvedRepositoryRoot,
      "crates",
      "providers",
      provider.packageName,
      "codepet-provider.json",
    );
    const sourceBinary = binaries.get(provider.name);
    await assertFile(sourceManifest, `${provider.name} Provider manifest`);
    await assertFile(sourceBinary, `${provider.name} Provider binary`);

    const manifest = JSON.parse(await readFile(sourceManifest, "utf8"));
    if (manifest.pluginId !== provider.pluginId) {
      throw new Error(
        `${provider.name} manifest pluginId must be ${provider.pluginId}, received ${manifest.pluginId}`,
      );
    }
    if (manifest.icon !== provider.icon) {
      throw new Error(
        `${provider.name} manifest icon must be ${provider.icon}, received ${manifest.icon}`,
      );
    }
    const executable = binaryFileName(provider, target);
    manifest.executable = executable;
    const destinationDirectory = path.join(resolvedStagingDirectory, provider.name);
    const destinationBinary = path.join(destinationDirectory, executable);
    await mkdir(destinationDirectory, { recursive: true });
    await copyFile(sourceBinary, destinationBinary);
    if (!isWindowsTarget(target)) {
      const metadata = await stat(destinationBinary);
      await chmod(destinationBinary, metadata.mode | 0o111);
    }
    await writeFile(
      path.join(destinationDirectory, "codepet-provider.json"),
      `${JSON.stringify(manifest, null, 2)}\n`,
    );
  }

  return resolvedStagingDirectory;
}

export async function stageProviderSdkResources({
  repositoryRoot = DEFAULT_REPOSITORY_ROOT,
  stagingDirectory = path.join(repositoryRoot, "src-tauri", "resources", "provider-sdk"),
}) {
  const directory=path.resolve(stagingDirectory);
  await mkdir(directory,{recursive:true});
  for (const entry of await readdir(directory)) {
    if (entry!==".gitkeep") await rm(path.join(directory,entry),{recursive:true,force:true});
  }
  return exportProviderSdk(directory,path.resolve(repositoryRoot));
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  const target =
    options.target ||
    process.env.CODEPET_PROVIDER_TARGET ||
    process.env.TAURI_ENV_TARGET_TRIPLE ||
    undefined;
  const binaries = await buildProviderBinaries(
    DEFAULT_REPOSITORY_ROOT,
    target,
    options.profile,
  );
  const stagingDirectory = await stageProviderPlugins({
    repositoryRoot: DEFAULT_REPOSITORY_ROOT,
    stagingDirectory: options.stagingDirectory,
    target,
    binaries,
  });
  const sdkStagingDirectory = await stageProviderSdkResources({
    repositoryRoot: DEFAULT_REPOSITORY_ROOT,
    target,
    binaries,
  });
  process.stdout.write(
    `staged ${PROVIDERS.length} Provider plugins in ${stagingDirectory}\n`
      + `staged Provider JSON Schema and author documentation in ${sdkStagingDirectory}\n`,
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error.stack || error.message}\n`);
    process.exitCode = 1;
  });
}
