import { createHash, randomUUID } from "node:crypto";
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contract = JSON.parse(readFileSync(join(projectRoot, "webcontent-contract.json"), "utf8"));
const manifestName = "manifest.json";
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

function assetPath(root, name) {
  if (!name || name.includes("\\") || name.includes(":") || name.includes("\0") || name.split("/").some((part) => !part || part === "." || part === "..")) {
    throw new Error(`Invalid webcontent path: ${name}`);
  }
  const path = realpathSync(join(root, name));
  const child = relative(root, path);
  if (!child || child.startsWith(`..${sep}`) || child === ".." || isAbsolute(child)) {
    throw new Error(`Asset escapes webcontent: ${name}`);
  }
  return path;
}

function validateBuildMetadata(manifest) {
  const identifier = '(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)';
  const semver = new RegExp(`^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)(?:-${identifier}(?:\\.${identifier})*)?(?:\\+[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*)?$`);
  if (typeof manifest.version !== 'string' || !semver.test(manifest.version)) throw new Error('Invalid webcontent semantic version');
  if (!Number.isSafeInteger(manifest.builtAt) || manifest.builtAt <= 0) throw new Error('Invalid webcontent build timestamp');
}

export function createManifest(directory, version, builtAt = Date.now()) {
  validateBuildMetadata({ version, builtAt });
  const root = realpathSync(directory);
  const files = {};
  function visit(directory, prefix = "") {
    for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const name = `${prefix}${entry.name}`;
      if (entry.isSymbolicLink()) throw new Error(`Symlinks are not supported: ${name}`);
      if (entry.isDirectory()) visit(join(directory, entry.name), `${name}/`);
      else if (entry.isFile() && name !== manifestName) files[name] = digest(readFileSync(assetPath(root, name)));
    }
  }
  visit(root);
  if (!files["index.html"] || !files["pet.html"]) throw new Error("webcontent requires index.html and pet.html");
  const manifest = { ...contract, version, builtAt, files };
  writeFileSync(join(root, manifestName), `${JSON.stringify(manifest, null, 2)}\n`);
  return manifest;
}

export function validateWebcontent(directory) {
  const root = realpathSync(directory);
  const manifest = JSON.parse(readFileSync(join(root, manifestName), "utf8"));
  for (const key of ["appId", "schemaVersion", "backendApiVersion"]) {
    if (manifest[key] !== contract[key]) throw new Error(`Incompatible webcontent ${key}: ${manifest[key]}`);
  }
  validateBuildMetadata(manifest);
  if (typeof manifest.files !== "object" || !manifest.files || Array.isArray(manifest.files)) throw new Error("Invalid webcontent manifest");
  for (const entry of ["index.html", "pet.html"]) {
    if (!Object.hasOwn(manifest.files, entry)) throw new Error(`Missing entry: ${entry}`);
  }
  for (const [name, expected] of Object.entries(manifest.files)) {
    if (!/^[a-f0-9]{64}$/.test(expected) || digest(readFileSync(assetPath(root, name))) !== expected) {
      throw new Error(`webcontent checksum mismatch: ${name}`);
    }
  }
  return manifest;
}

// Install into a new data-directory version. Existing versions and App resources
// are never overwritten; incomplete staging directories are not loader candidates.
export function installWebcontent(source, dataDirectory) {
  const sourceRoot = realpathSync(source);
  const manifest = validateWebcontent(sourceRoot);
  const dataRoot = resolve(dataDirectory);
  let ancestor = dataRoot;
  while (!existsSync(ancestor) && dirname(ancestor) !== ancestor) ancestor = dirname(ancestor);
  const physicalDataRoot = resolve(realpathSync(ancestor), relative(ancestor, dataRoot));
  if ([dataRoot, physicalDataRoot].some((path) => path.split(sep).some((part) => part.toLowerCase().endsWith(".app")))) {
    throw new Error("Install into the application data directory, not a signed App bundle");
  }
  const versionRoot = join(dataRoot, "versions");
  if (existsSync(versionRoot) && lstatSync(versionRoot).isSymbolicLink()) throw new Error("Version directory must not be a symlink");
  const versions = join(dataRoot, "versions", "webcontent");
  const contains = (path) => !path || (!isAbsolute(path) && path !== ".." && !path.startsWith(`..${sep}`));
  if (contains(relative(sourceRoot, versions)) || contains(relative(versions, sourceRoot))) {
    throw new Error("Source and destination must not overlap");
  }
  if (existsSync(versions) && lstatSync(versions).isSymbolicLink()) throw new Error("Version directory must not be a symlink");
  const physicalVersions = join(physicalDataRoot, "versions", "webcontent");
  if (contains(relative(sourceRoot, physicalVersions)) || contains(relative(physicalVersions, sourceRoot))) {
    throw new Error("Source and destination must not overlap through a symlink");
  }
  mkdirSync(versions, { recursive: true });
  const parent = realpathSync(versions);
  let latest = 0n;
  const maxVersion = 18446744073709551615n;
  for (const entry of readdirSync(parent, { withFileTypes: true })) {
    if (!/^v[1-9][0-9]*$/.test(entry.name)) continue;
    const version = BigInt(entry.name.slice(1));
    if (version > maxVersion) continue;
    if (entry.isSymbolicLink()) throw new Error(`Version must not be a symlink: ${entry.name}`);
    if (entry.isDirectory() && version > latest) latest = version;
  }
  if (latest === maxVersion) throw new Error("webcontent version limit reached");
  const directoryVersion = `v${latest + 1n}`;
  const target = join(parent, directoryVersion);
  const staging = join(parent, `.webcontent-stage-${randomUUID()}`);
  mkdirSync(staging);
  try {
    for (const name of Object.keys(manifest.files)) {
      const output = join(staging, name);
      mkdirSync(dirname(output), { recursive: true });
      writeFileSync(output, readFileSync(assetPath(sourceRoot, name)));
    }
    writeFileSync(join(staging, manifestName), `${JSON.stringify(manifest, null, 2)}\n`);
    validateWebcontent(staging);
    if (existsSync(target)) throw new Error(`Version already exists: ${target}; retry installation`);
    renameSync(staging, target);
    return { target, directoryVersion, version: manifest.version, builtAt: manifest.builtAt };
  } finally {
    // Only delete the staging folder allocated by this invocation in the named data root.
    if (dirname(staging) === parent && basename(staging).startsWith(".webcontent-stage-") && existsSync(staging)) {
      rmSync(staging, { recursive: true });
    }
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    const [command, ...args] = process.argv.slice(2);
    const source = join(projectRoot, "webcontent");
    if (command === "manifest" && args.length === 0) {
      const { version } = JSON.parse(readFileSync(join(projectRoot, "package.json"), "utf8"));
      const build = JSON.parse(readFileSync(join(source, "build-info.json"), "utf8"));
      if (build.version !== version) throw new Error("Build metadata version differs from package.json; rebuild webcontent");
      const manifest = createManifest(source, version, build.builtAt);
      console.log(`Built webcontent ${version}: ${Object.keys(manifest.files).length} files; backend API ${manifest.backendApiVersion}`);
    } else if (command === "install" && args.length === 2 && args[0] === "--data-dir") {
      console.log(JSON.stringify(installWebcontent(source, args[1]), null, 2));
    } else {
      throw new Error('Usage: npm run build:webcontent | npm run install:webcontent -- --data-dir "<app-data-dir>" (restart the App after installation)');
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
