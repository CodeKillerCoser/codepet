import { appendFileSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const semverPattern =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

export function prepareReleaseMetadata(options = {}) {
  const versions = readReleaseVersions();
  validateReleaseVersions(versions);

  const inputVersion = firstValue(options.version, process.env.INPUT_VERSION);
  const baseVersion = normalizeBaseVersion(inputVersion ?? versions.tauri, inputVersion ? "release version input" : "source version");
  const commitSha = normalizeCommitSha(firstValue(options.commitSha, process.env.RELEASE_COMMIT_SHA));
  const releaseVersion = commitSha ? buildVersionWithCommit(baseVersion, commitSha) : baseVersion;
  const tag = normalizeReleaseTag(firstValue(options.tag, process.env.INPUT_TAG), releaseVersion, baseVersion);
  const releaseName = firstValue(options.releaseName, process.env.INPUT_RELEASE_NAME) ?? `Release ${tag}`;
  const releaseNotes = firstValue(options.releaseNotes, process.env.INPUT_RELEASE_NOTES) ?? releaseName;

  validateTagMatchesVersion(tag, releaseVersion);

  return {
    baseVersion,
    version: releaseVersion,
    commitSha: commitSha ?? "",
    tag,
    releaseName,
    releaseNotes,
    versions,
  };
}

export function readReleaseVersions() {
  const packageLock = readJson(resolve(root, "package-lock.json"));
  const cargoLockVersion = readCargoLockVersion(resolve(root, "src-tauri", "Cargo.lock"));
  const versions = {
    tauri: readJson(resolve(root, "src-tauri", "tauri.conf.json")).version,
    packageJson: readJson(resolve(root, "package.json")).version,
    packageLock: packageLock.version,
    packageLockPackage: packageLock.packages?.[""]?.version,
    cargoToml: readCargoTomlVersion(resolve(root, "src-tauri", "Cargo.toml")),
  };

  if (cargoLockVersion) {
    versions.cargoLock = cargoLockVersion;
  }

  return versions;
}

export function writeReleaseVersions(version, options = {}) {
  const normalizedVersion = normalizeVersion(version, {
    allowBuildMetadata: options.allowBuildMetadata === true,
    source: "release version",
  });

  writeJson(resolve(root, "package.json"), (json) => {
    json.version = normalizedVersion;
  });

  writeJson(resolve(root, "package-lock.json"), (json) => {
    json.version = normalizedVersion;
    if (json.packages?.[""]) {
      json.packages[""].version = normalizedVersion;
    }
  });

  const tauriConfigPath = resolve(root, "src-tauri", "tauri.conf.json");
  writeFileSync(tauriConfigPath, replaceJsonRootVersion(readFileSync(tauriConfigPath, "utf8"), normalizedVersion));

  const cargoTomlPath = resolve(root, "src-tauri", "Cargo.toml");
  writeFileSync(cargoTomlPath, replaceCargoTomlVersion(readFileSync(cargoTomlPath, "utf8"), normalizedVersion));

  const cargoLockPath = resolve(root, "src-tauri", "Cargo.lock");
  writeFileSync(cargoLockPath, replaceCargoLockVersion(readFileSync(cargoLockPath, "utf8"), normalizedVersion));

  validateReleaseVersions(readReleaseVersions(), { allowBuildMetadata: options.allowBuildMetadata === true });

  return normalizedVersion;
}

export function buildVersionWithCommit(baseVersion, commitSha) {
  const normalizedBaseVersion = normalizeBaseVersion(baseVersion, "base version");
  const normalizedCommitSha = normalizeCommitSha(commitSha);
  return `${normalizedBaseVersion}+${normalizedCommitSha.slice(0, 7)}`;
}

export function normalizeBaseVersion(version, source = "version") {
  return normalizeVersion(version, { allowBuildMetadata: false, source });
}

export function validateReleaseVersions(versions, options = {}) {
  const expected = versions.tauri;
  const mismatches = Object.entries(versions)
    .filter(([, version]) => version !== expected)
    .map(([source, version]) => `${source}: ${version ?? "<missing>"}`);

  if (!expected) {
    fail("Missing src-tauri/tauri.conf.json version.");
  }

  normalizeVersion(expected, {
    allowBuildMetadata: options.allowBuildMetadata === true,
    source: "src-tauri/tauri.conf.json version",
  });

  if (mismatches.length > 0) {
    fail(
      [
        "Release version sources must match src-tauri/tauri.conf.json.",
        `Expected: ${expected}`,
        ...mismatches,
        "Use scripts/release_version.mjs --sync-version <version> to update all release version sources together.",
      ].join("\n"),
    );
  }
}

export function validateTagMatchesVersion(releaseTag, releaseVersion) {
  const tagVersion = releaseTag.replace(/^v/, "");
  if (tagVersion !== releaseVersion) {
    fail(
      [
        "Release tag must match the Tauri app version.",
        `Tag: ${releaseTag}`,
        `Version: ${releaseVersion}`,
        "For normal releases, leave the workflow tag input empty and let CI derive v<version+short_commit>.",
      ].join("\n"),
    );
  }
}

function normalizeVersion(value, options = {}) {
  const version = value?.trim();
  if (!version) {
    fail(`Missing ${options.source ?? "version"}.`);
  }

  const match = version.match(semverPattern);
  if (!match) {
    fail(`${options.source ?? "Version"} must be a valid semantic version: ${version}`);
  }

  if (!options.allowBuildMetadata && match[5]) {
    fail(
      [
        `${options.source ?? "Version"} must be a base semantic version without build metadata: ${version}`,
        "CI appends +<short_commit> at build time so the committed source version stays stable.",
      ].join("\n"),
    );
  }

  return version;
}

function normalizeCommitSha(value) {
  const commitSha = value?.trim();
  if (!commitSha) {
    return undefined;
  }

  if (!/^[0-9a-f]{7,40}$/i.test(commitSha)) {
    fail(`Commit SHA must be a 7-40 character hexadecimal Git object id: ${commitSha}`);
  }

  return commitSha.toLowerCase();
}

function normalizeReleaseTag(value, version, baseVersion) {
  const tag = value?.trim();
  if (!tag) {
    return `v${version}`;
  }

  const tagVersion = tag.replace(/^v/, "");
  if (tagVersion === baseVersion) {
    return `v${version}`;
  }

  return tag;
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function writeJson(path, update) {
  const json = readJson(path);
  update(json);
  writeFileSync(path, `${JSON.stringify(json, null, 2)}\n`);
}

function replaceJsonRootVersion(text, version) {
  if (!/"version"\s*:\s*"[^"]+"/.test(text)) {
    fail("Missing version in src-tauri/tauri.conf.json.");
  }

  return text.replace(/("version"\s*:\s*)"[^"]+"/, `$1"${version}"`);
}

function replaceCargoTomlVersion(text, version) {
  let inPackage = false;
  let replaced = false;
  const lines = text.split(/\r?\n/).map((line) => {
    const section = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (section) {
      inPackage = section[1] === "package";
      return line;
    }

    if (inPackage && /^\s*version\s*=\s*"[^"]+"/.test(line)) {
      replaced = true;
      return line.replace(/^(\s*version\s*=\s*)"[^"]+"/, `$1"${version}"`);
    }

    return line;
  });

  if (!replaced) {
    fail("Missing [package] version in src-tauri/Cargo.toml.");
  }

  return lines.join("\n");
}

function replaceCargoLockVersion(text, version) {
  let replaced = false;
  const updated = text.replace(/\[\[package\]\]\r?\n([\s\S]*?)(?=\r?\n\[\[package\]\]|\s*$)/g, (block, body) => {
    if (!/^\s*name\s*=\s*"code-pet"\s*$/m.test(body)) {
      return block;
    }

    replaced = true;
    return block.replace(/^(\s*version\s*=\s*)"[^"]+"/m, `$1"${version}"`);
  });

  return replaced ? updated : text;
}

function readCargoTomlVersion(path) {
  const lines = readFileSync(path, "utf8").split(/\r?\n/);
  let inPackage = false;
  for (const line of lines) {
    const section = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (section) {
      inPackage = section[1] === "package";
      continue;
    }

    if (inPackage) {
      const version = line.match(/^\s*version\s*=\s*"([^"]+)"/);
      if (version) {
        return version[1];
      }
    }
  }
  return undefined;
}

function readCargoLockVersion(path) {
  const text = readFileSync(path, "utf8");
  const packageBlocks = text.matchAll(/\[\[package\]\]\r?\n([\s\S]*?)(?=\r?\n\[\[package\]\]|\s*$)/g);
  for (const block of packageBlocks) {
    if (!/^\s*name\s*=\s*"code-pet"\s*$/m.test(block[1])) {
      continue;
    }

    return block[1].match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1];
  }
  return undefined;
}

function writeGitHubOutput(path, metadata) {
  const lines = [
    `base_version=${metadata.baseVersion}`,
    `version=${metadata.version}`,
    `commit_sha=${metadata.commitSha}`,
    `tag=${metadata.tag}`,
    `release_name=${metadata.releaseName}`,
    multilineOutput("release_notes", metadata.releaseNotes),
  ];
  appendFileSync(path, `${lines.join("\n")}\n`);
}

function multilineOutput(name, value) {
  const delimiter = `${name.toUpperCase()}_${Date.now()}_${Math.random().toString(16).slice(2)}`;
  return `${name}<<${delimiter}\n${value}\n${delimiter}`;
}

function firstValue(...values) {
  return values.find((value) => typeof value === "string" && value.trim() !== "");
}

function parseArgs(argv) {
  const parsed = {};
  const booleanArgs = new Set(["allowBuildMetadata", "printVersion"]);
  for (let index = 0; index < argv.length; index += 1) {
    const item = argv[index];
    if (!item.startsWith("--")) {
      fail(`Unexpected argument: ${item}`);
    }

    const [rawKey, inlineValue] = item.slice(2).split("=", 2);
    const key = rawKey.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
    if (inlineValue !== undefined) {
      parsed[key] = inlineValue;
      continue;
    }

    const nextValue = argv[index + 1];
    if (booleanArgs.has(key) && (!nextValue || nextValue.startsWith("--"))) {
      parsed[key] = true;
      continue;
    }

    if (!nextValue || nextValue.startsWith("--")) {
      fail(`Missing value for --${rawKey}`);
    }

    parsed[key] = nextValue;
    index += 1;
  }

  return parsed;
}

function isMainModule() {
  if (!process.argv[1]) {
    return false;
  }

  const invokedPath = normalizeComparablePath(resolve(process.argv[1]));
  const modulePath = normalizeComparablePath(fileURLToPath(import.meta.url));
  return invokedPath === modulePath;
}

function normalizeComparablePath(path) {
  return realpathSync(path).replaceAll("\\", "/").toLowerCase();
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

if (isMainModule()) {
  const args = parseArgs(process.argv.slice(2));

  if (args.syncVersion) {
    const syncedVersion = writeReleaseVersions(args.syncVersion, {
      allowBuildMetadata: args.allowBuildMetadata === true,
    });
    console.log(`Synced release version: ${syncedVersion}`);

    const syncOnly =
      !args.printVersion &&
      !args.githubOutput &&
      !args.version &&
      !args.commitSha &&
      !args.tag &&
      !args.releaseName &&
      !args.releaseNotes;
    if (syncOnly) {
      process.exit(0);
    }
  }

  if (args.printVersion === true) {
    const versions = readReleaseVersions();
    validateReleaseVersions(versions, { allowBuildMetadata: args.allowBuildMetadata === true });
    console.log(versions.tauri);
    process.exit(0);
  }

  const metadata = prepareReleaseMetadata({
    tag: args.tag,
    releaseName: args.releaseName,
    releaseNotes: args.releaseNotes,
    version: args.version,
    commitSha: args.commitSha,
  });

  if (args.githubOutput) {
    writeGitHubOutput(resolve(root, args.githubOutput), metadata);
  } else {
    console.log(`Release base version: ${metadata.baseVersion}`);
    console.log(`Release build version: ${metadata.version}`);
    console.log(`Release tag: ${metadata.tag}`);
    console.log("Version sources are consistent.");
  }
}
