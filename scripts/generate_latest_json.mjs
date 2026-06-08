import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const args = parseArgs(process.argv.slice(2));
const root = fileURLToPath(new URL("..", import.meta.url));
const tauriConfigPath = resolve(root, "src-tauri", "tauri.conf.json");
const tauriConfig = readJson(tauriConfigPath);
const version = args.version ?? tauriConfig.version;
const tag = args.tag ?? `v${version}`;
const repository = normalizeRepository(args.repo ?? process.env.GITHUB_REPOSITORY ?? readGitRemote());
const artifactsDir = resolve(root, args.artifacts ?? "dist/release-assets");
const outputPath = resolve(root, args.output ?? "dist/latest.json");
const notes = args.notes ?? `Release ${version}`;
const pubDate = args.pubDate ?? new Date().toISOString();
const updaterManifestName = "latest.json";
const updaterManifestEndpoint = `https://github.com/${repository}/releases/latest/download/${updaterManifestName}`;

if (!version) {
  fail("Missing release version. Pass --version or set src-tauri/tauri.conf.json version.");
}

validateUpdaterEndpoint(tauriConfig, updaterManifestEndpoint);

if (!existsSync(artifactsDir)) {
  fail(`Artifacts directory does not exist: ${artifactsDir}`);
}

const files = walkFiles(artifactsDir).sort((left, right) => left.localeCompare(right));
const macArchive = findArtifact(files, (file) => file.endsWith(".app.tar.gz"), "macOS .app.tar.gz updater archive");
const windowsInstaller = findArtifact(
  files,
  (file) => {
    const name = basename(file).toLowerCase();
    return name.endsWith(".exe") && name.includes("setup");
  },
  "Windows NSIS setup .exe updater artifact",
);

const latest = {
  version,
  notes,
  pub_date: pubDate,
  platforms: {
    "macos-universal": releaseAsset(macArchive),
    "windows-x86_64": releaseAsset(windowsInstaller),
  },
};

mkdirSync(dirname(outputPath), { recursive: true });
writeFileSync(outputPath, `${JSON.stringify(latest, null, 2)}\n`);
console.log(`Wrote ${outputPath}`);
console.log(`Updater manifest endpoint: ${updaterManifestEndpoint}`);

function releaseAsset(file) {
  const signaturePath = `${file}.sig`;
  if (!existsSync(signaturePath)) {
    fail(`Missing signature for ${file}. Expected ${signaturePath}`);
  }

  return {
    signature: readFileSync(signaturePath, "utf8").trim(),
    url: `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(githubReleaseAssetName(file))}`,
  };
}

function githubReleaseAssetName(file) {
  return basename(file).replaceAll(" ", ".");
}

function validateUpdaterEndpoint(config, expectedEndpoint) {
  const endpoints = config.plugins?.updater?.endpoints;
  if (!Array.isArray(endpoints) || endpoints.length === 0) {
    fail("Missing Tauri updater endpoint in src-tauri/tauri.conf.json.");
  }

  if (endpoints.length !== 1 || endpoints[0] !== expectedEndpoint) {
    fail(
      [
        "Tauri updater endpoint must be the stable latest.json URL.",
        `Expected exactly: ${expectedEndpoint}`,
        `Found: ${endpoints.join(", ")}`,
        "Do not use a tag-scoped /releases/download/<tag>/latest.json URL as the updater endpoint.",
      ].join("\n"),
    );
  }
}

function findArtifact(files, predicate, description) {
  const matches = files.filter((file) => predicate(file));
  if (matches.length === 0) {
    fail(`Could not find ${description} in ${artifactsDir}`);
  }

  if (matches.length > 1) {
    console.warn(`Multiple candidates found for ${description}; using ${matches[0]}`);
  }

  return matches[0];
}

function walkFiles(directory) {
  return readdirSync(directory).flatMap((entry) => {
    const entryPath = join(directory, entry);
    const stats = statSync(entryPath);
    return stats.isDirectory() ? walkFiles(entryPath) : [entryPath];
  });
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function readGitRemote() {
  try {
    return execFileSync("git", ["config", "--get", "remote.origin.url"], {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {
    return "";
  }
}

function normalizeRepository(value) {
  const repository = value
    .replace(/^git@github\.com:/, "")
    .replace(/^https:\/\/github\.com\//, "")
    .replace(/\.git$/, "")
    .trim();

  if (!/^[^/\s]+\/[^/\s]+$/.test(repository)) {
    fail("Missing GitHub repository. Pass --repo owner/name or run inside a GitHub-backed checkout.");
  }

  return repository;
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

function fail(message) {
  console.error(message);
  process.exit(1);
}
