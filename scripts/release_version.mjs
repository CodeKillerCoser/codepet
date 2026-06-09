import { appendFileSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));

export function prepareReleaseMetadata(options = {}) {
  const versions = readReleaseVersions();
  const releaseVersion = versions.tauri;
  const tag = normalizeReleaseTag(firstValue(options.tag, process.env.INPUT_TAG), releaseVersion);
  const releaseName = firstValue(options.releaseName, process.env.INPUT_RELEASE_NAME) ?? `Release ${tag}`;
  const releaseNotes = firstValue(options.releaseNotes, process.env.INPUT_RELEASE_NOTES) ?? releaseName;

  validateReleaseVersions(versions);
  validateTagMatchesVersion(tag, releaseVersion);

  return {
    version: releaseVersion,
    tag,
    releaseName,
    releaseNotes,
    versions,
  };
}

export function readReleaseVersions() {
  return {
    tauri: readJson(resolve(root, "src-tauri", "tauri.conf.json")).version,
    packageJson: readJson(resolve(root, "package.json")).version,
    packageLock: readJson(resolve(root, "package-lock.json")).packages?.[""]?.version,
    cargoToml: readCargoTomlVersion(resolve(root, "src-tauri", "Cargo.toml")),
    cargoLock: readCargoLockVersion(resolve(root, "src-tauri", "Cargo.lock")),
  };
}

export function validateReleaseVersions(versions) {
  const expected = versions.tauri;
  const mismatches = Object.entries(versions)
    .filter(([, version]) => version !== expected)
    .map(([source, version]) => `${source}: ${version ?? "<missing>"}`);

  if (!expected) {
    fail("Missing src-tauri/tauri.conf.json version.");
  }

  if (mismatches.length > 0) {
    fail(
      [
        "Release version sources must match src-tauri/tauri.conf.json.",
        `Expected: ${expected}`,
        ...mismatches,
        "Run npm run version:check before publishing, and bump all version files in the same commit.",
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
        "Do not use the workflow tag input for normal releases; let the workflow derive v<version> from source.",
      ].join("\n"),
    );
  }
}

function normalizeReleaseTag(value, version) {
  const tag = value?.trim();
  return tag ? tag : `v${version}`;
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
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
    `version=${metadata.version}`,
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
  for (let index = 0; index < argv.length; index += 1) {
    const item = argv[index];
    if (!item.startsWith("--")) {
      fail(`Unexpected argument: ${item}`);
    }

    const [rawKey, inlineValue] = item.slice(2).split("=", 2);
    const key = rawKey.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
    const value = inlineValue ?? argv[index + 1];
    if (!value || value.startsWith("--")) {
      fail(`Missing value for --${rawKey}`);
    }

    parsed[key] = value;
    if (inlineValue === undefined) {
      index += 1;
    }
  }

  return parsed;
}

function isMainModule() {
  return process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

if (isMainModule()) {
  const args = parseArgs(process.argv.slice(2));
  const metadata = prepareReleaseMetadata({
    tag: args.tag,
    releaseName: args.releaseName,
    releaseNotes: args.releaseNotes,
  });

  if (args.githubOutput) {
    writeGitHubOutput(resolve(root, args.githubOutput), metadata);
  } else {
    console.log(`Release version: ${metadata.version}`);
    console.log(`Release tag: ${metadata.tag}`);
    console.log("Version sources are consistent.");
  }
}
