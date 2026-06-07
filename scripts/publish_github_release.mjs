import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const args = parseArgs(process.argv.slice(2));
const root = fileURLToPath(new URL("..", import.meta.url));
const tauriConfig = JSON.parse(readFileSync(resolve(root, "src-tauri", "tauri.conf.json"), "utf8"));
const tag = args.tag ?? `v${tauriConfig.version}`;
const ref = args.ref ?? "main";
const releaseName = args.releaseName ?? `Release ${tag}`;
const releaseNotes = args.releaseNotes ?? releaseName;

run("gh", [
  "workflow",
  "run",
  "release.yml",
  "--ref",
  ref,
  "--field",
  `tag=${tag}`,
  "--field",
  `release_name=${releaseName}`,
  "--field",
  `release_notes=${releaseNotes}`,
]);

console.log(`Queued GitHub release workflow for ${tag} on ${ref}.`);

function run(command, commandArgs) {
  const result = spawnSync(command, commandArgs, {
    cwd: root,
    stdio: "inherit",
    shell: process.platform === "win32",
  });

  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
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
