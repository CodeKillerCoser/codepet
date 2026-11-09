import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { normalizeBaseVersion } from "./release_version.mjs";

export function workflowArguments(mode, argv) {
  const fields = mode === "build" ? ["platform", "version"] : ["build-run-id", "release-name", "release-notes"];
  const { values } = parseArgs({ args: argv, options: Object.fromEntries(["ref", ...fields].map(key => [key, { type: "string" }])) });
  if (mode === "build") {
    values.platform ??= "all";
    if (!["all", "mac", "win"].includes(values.platform)) throw new Error("--platform must be all, mac, or win.");
    if (values.version) normalizeBaseVersion(values.version);
  } else if (!/^[1-9]\d*$/.test(values["build-run-id"] ?? "")) {
    throw new Error("Pass --build-run-id <successful Build run ID>. Release no longer builds from source.");
  }
  return ["workflow", "run", `${mode}.yml`, "--ref", values.ref ?? "main",
    ...fields.filter(key => values[key] !== undefined).flatMap(key => ["--raw-field", `${key.replaceAll("-", "_")}=${values[key]}`])];
}

export function dispatchWorkflow(mode, argv) {
  try {
    const args = workflowArguments(mode, argv);
    const result = spawnSync("gh", args, { cwd: fileURLToPath(new URL("..", import.meta.url)), stdio: "inherit" });
    if (result.error) throw result.error;
    if (result.status !== 0) { process.exitCode = result.status ?? 1; return; }
    console.log(`Queued GitHub ${mode} workflow.`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  dispatchWorkflow("release", process.argv.slice(2));
}
