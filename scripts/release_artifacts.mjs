import { appendFileSync, copyFileSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { buildVersionWithCommit, normalizeBaseVersion } from "./release_version.mjs";

const platformSets = { all: ["macos-arm64", "windows-x86_64"], mac: ["macos-arm64"], win: ["windows-x86_64"] };

export function buildMetadata({ baseVersion, commitSha, platform, runId }) {
  if (!/^[0-9a-f]{40}$/.test(commitSha ?? "")) throw new Error("A full lowercase source commit SHA is required.");
  if (!Object.hasOwn(platformSets, platform)) throw new Error("Platform must be all, mac, or win.");
  if (!/^[1-9]\d*$/.test(String(runId))) throw new Error("A build run ID is required.");
  normalizeBaseVersion(baseVersion);
  const version = buildVersionWithCommit(baseVersion, commitSha);
  return { schemaVersion: 1, baseVersion, version, commitSha, tag: `v${version}`, platform, runId: String(runId) };
}

export function validateMetadata(value) {
  const expected = buildMetadata(value);
  for (const key of Object.keys(expected)) {
    if (value[key] !== expected[key]) throw new Error(`Invalid build metadata: ${key}`);
  }
  return expected;
}

export function artifactNames(metadata) {
  const m = validateMetadata(metadata);
  const suffix = `${m.baseVersion}_${m.commitSha.slice(0, 7)}`;
  return {
    metadata: `codepet-build-${m.platform}_${suffix}.json`,
    "macos-arm64": `codepet-macos-arm64_${suffix}.dmg`,
    "windows-x86_64": `codepet-windows-x86_64_${suffix}-setup.exe`,
  };
}

// Rename only the installer container, leaving signed bytes and paired signatures intact.
export function stageArtifacts(metadata, platform, input, output) {
  const names = artifactNames(metadata);
  if (!platformSets[metadata.platform].includes(platform)) throw new Error("Platform was not selected for this build.");
  const extension = platform === "macos-arm64" ? ".dmg" : "-setup.exe";
  const candidates = readdirSync(input).filter(name => name.endsWith(extension));
  if (candidates.length !== 1) throw new Error(`Expected one ${extension} installer, found ${candidates.length}.`);
  const source = candidates[0];
  const architecture = platform === "macos-arm64" ? "aarch64" : "x64";
  if (!["Code Pet", "Code.Pet"].some(prefix => source === `${prefix}_${metadata.version}_${architecture}${extension}`)) {
    throw new Error(`Installer name does not match build version and architecture: ${source}`);
  }
  const pairs = [[source, names[platform]]];
  if (platform === "windows-x86_64") pairs.push([`${source}.sig`, `${names[platform]}.sig`]);
  for (const [from] of pairs) {
    if (!statSync(join(input, from)).isFile() || statSync(join(input, from)).size === 0) throw new Error(`Empty or invalid artifact: ${from}`);
    if (from.endsWith(".sig") && !readFileSync(join(input, from), "utf8").trim()) throw new Error("Empty updater signature.");
  }
  mkdirSync(output, { recursive: true });
  for (const [from, to] of pairs) copyFileSync(join(input, from), join(output, to));
  return names[platform];
}

export function validateBuildRun(run, runId, repository) {
  if (String(run.id) !== String(runId) || run.repository?.full_name !== repository
      || run.path !== ".github/workflows/build.yml" || run.event !== "workflow_dispatch"
      || run.status !== "completed" || run.conclusion !== "success") {
    throw new Error("Release requires a successful completed Build run from this repository.");
  }
}

export function validateArtifacts(directory, runId) {
  const files = readdirSync(directory).sort();
  const manifests = files.filter(name => /^codepet-build-.*\.json$/.test(name));
  if (manifests.length !== 1) throw new Error("Expected exactly one build metadata file.");
  const metadata = validateMetadata(JSON.parse(readFileSync(join(directory, manifests[0]), "utf8")));
  if (metadata.runId !== String(runId)) throw new Error("Build run ID does not match artifact metadata.");
  const names = artifactNames(metadata);
  const expected = [names.metadata];
  for (const platform of platformSets[metadata.platform]) {
    expected.push(names[platform]);
    if (platform === "windows-x86_64") expected.push(`${names[platform]}.sig`);
  }
  if (JSON.stringify(files) !== JSON.stringify(expected.sort())) throw new Error("Missing, unexpected, or mixed-build artifacts.");
  for (const file of files) {
    if (!statSync(join(directory, file)).isFile() || statSync(join(directory, file)).size === 0) throw new Error(`Empty or invalid artifact: ${file}`);
    if (file.endsWith(".sig") && !readFileSync(join(directory, file), "utf8").trim()) throw new Error("Empty updater signature.");
  }
  return metadata;
}

function outputs(values) {
  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(values).map(([key, value]) => `${key}=${value}\n`).join(""));
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const { values: args, positionals: [command] } = parseArgs({ allowPositionals: true, options: Object.fromEntries(
      ["version", "commit", "platform", "run-id", "input", "output", "metadata", "run-json", "repo"].map(key => [key, { type: "string" }]),
    ) });
    if (command === "metadata") {
      const metadata = buildMetadata({ baseVersion: args.version, commitSha: args.commit, platform: args.platform, runId: args["run-id"] });
      const names = artifactNames(metadata);
      mkdirSync(args.output, { recursive: true });
      writeFileSync(join(args.output, names.metadata), `${JSON.stringify(metadata, null, 2)}\n`);
      outputs({ metadata: join(args.output, names.metadata), metadata_name: names.metadata, mac_name: names["macos-arm64"], win_name: names["windows-x86_64"] });
    } else if (command === "stage") {
      const metadata = validateMetadata(JSON.parse(readFileSync(args.metadata, "utf8")));
      stageArtifacts(metadata, args.platform, args.input, args.output);
    } else if (command === "validate") {
      validateBuildRun(JSON.parse(readFileSync(args["run-json"], "utf8")), args["run-id"], args.repo);
      const metadata = validateArtifacts(args.input, args["run-id"]);
      outputs({ version: metadata.version, tag: metadata.tag, commit_sha: metadata.commitSha, platform: metadata.platform });
      console.log(`Validated ${metadata.tag} from Build ${metadata.runId} (${metadata.commitSha}).`);
    } else {
      throw new Error("Expected metadata, stage, or validate command.");
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
