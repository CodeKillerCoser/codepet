#!/usr/bin/env node

import { chmod, copyFile, mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

export const PROVIDERS = Object.freeze([
  {
    name: "codex",
    packageName: "codepet-provider-codex",
    pluginId: "dev.codepet.codex",
  },
  {
    name: "opencode",
    packageName: "codepet-provider-opencode",
    pluginId: "dev.codepet.opencode",
  },
  {
    name: "claude",
    packageName: "codepet-provider-claude",
    pluginId: "dev.codepet.claude",
  },
]);

const SCRIPT_DIRECTORY = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_REPOSITORY_ROOT = path.resolve(SCRIPT_DIRECTORY, "..");
const UNIVERSAL_MACOS_TARGET = "universal-apple-darwin";

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--target") {
      options.target = argv[index + 1];
      index += 1;
    } else if (argument === "--staging-dir") {
      options.stagingDirectory = argv[index + 1];
      index += 1;
    } else {
      throw new Error(`unknown argument: ${argument}`);
    }
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

function cargoBuild(repositoryRoot, target) {
  const args = [
    "build",
    "--manifest-path",
    path.join(repositoryRoot, "crates", "Cargo.toml"),
    "--release",
  ];
  if (target) {
    args.push("--target", target);
  }
  for (const provider of PROVIDERS) {
    args.push("--package", provider.packageName, "--bin", provider.packageName);
  }
  run(process.env.CARGO || "cargo", args, repositoryRoot);
}

function releaseBinaryPath(repositoryRoot, provider, target) {
  const targetDirectory = target
    ? path.join(repositoryRoot, "crates", "target", target, "release")
    : path.join(repositoryRoot, "crates", "target", "release");
  return path.join(targetDirectory, binaryFileName(provider, target));
}

async function assertFile(filePath, description) {
  const metadata = await stat(filePath).catch(() => null);
  if (!metadata?.isFile()) {
    throw new Error(`${description} is missing: ${filePath}`);
  }
}

async function buildProviderBinaries(repositoryRoot, target) {
  if (target === UNIVERSAL_MACOS_TARGET) {
    if (process.platform !== "darwin") {
      throw new Error("universal macOS Provider binaries can only be built on macOS");
    }
    const architectureTargets = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
    for (const architectureTarget of architectureTargets) {
      cargoBuild(repositoryRoot, architectureTarget);
    }
    const universalDirectory = path.join(
      repositoryRoot,
      "crates",
      "target",
      UNIVERSAL_MACOS_TARGET,
      "release",
    );
    await mkdir(universalDirectory, { recursive: true });
    const binaries = new Map();
    for (const provider of PROVIDERS) {
      const inputs = architectureTargets.map((architectureTarget) =>
        releaseBinaryPath(repositoryRoot, provider, architectureTarget),
      );
      for (const input of inputs) {
        await assertFile(input, `${provider.name} architecture binary`);
      }
      const output = path.join(universalDirectory, binaryFileName(provider, target));
      run("lipo", ["-create", ...inputs, "-output", output], repositoryRoot);
      binaries.set(provider.name, output);
    }
    return binaries;
  }

  cargoBuild(repositoryRoot, target);
  return new Map(
    PROVIDERS.map((provider) => [
      provider.name,
      releaseBinaryPath(repositoryRoot, provider, target),
    ]),
  );
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
    if (entry !== ".gitkeep") {
      await rm(path.join(resolvedStagingDirectory, entry), { recursive: true, force: true });
    }
  }
  await writeFile(path.join(resolvedStagingDirectory, ".gitkeep"), "");

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

async function main() {
  const options = parseArguments(process.argv.slice(2));
  const target =
    options.target ||
    process.env.CODEPET_PROVIDER_TARGET ||
    process.env.TAURI_ENV_TARGET_TRIPLE ||
    undefined;
  const binaries = await buildProviderBinaries(DEFAULT_REPOSITORY_ROOT, target);
  const stagingDirectory = await stageProviderPlugins({
    repositoryRoot: DEFAULT_REPOSITORY_ROOT,
    stagingDirectory: options.stagingDirectory,
    target,
    binaries,
  });
  process.stdout.write(`staged ${PROVIDERS.length} Provider plugins in ${stagingDirectory}\n`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`${error.stack || error.message}\n`);
    process.exitCode = 1;
  });
}
