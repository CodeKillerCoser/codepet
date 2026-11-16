import { mkdir, cp, writeFile, rm } from "node:fs/promises";
import path from "node:path";
import { packageIcons } from "./package-icons.mjs";
import { validatePluginManifest } from "./contracts.mjs";
import { fileURLToPath } from "node:url";
const root = path.dirname(fileURLToPath(import.meta.url));
const get = (name, defaultValue) => {
  const i = process.argv.indexOf(`--${name}`);
  return i < 0 ? defaultValue : process.argv[i + 1];
};
const transport = get("transport", "stdio-jsonrpc-v1"),
  pluginId = get("plugin-id", "independent.echo.custom"),
  instanceId = get("instance-id", "independent-echo-custom"),
  backend = get("backend", "echo");
if (get("protocol-version", "4") !== "4")
  throw Error("business protocol must be 4/4");
const out = path.resolve(get("out", "packages/custom"));
const manifest = {
  manifestVersion: 2,
  pluginId,
  displayName:
    backend === "notice"
      ? "Independent Notice fixture"
      : backend === "pi"
        ? "Independent Pi RPC"
        : "Independent Echo",
  icon: get("icon", packageIcons[backend === "notice" ? "echo" : backend]),
  command: get("node", "node"),
  args: ["provider.mjs"],
  env: {
    ECHO_PLUGIN_ID: pluginId,
    ...(backend !== "echo" ? { ECHO_BACKEND: backend } : {}),
    ...(get("protocol-version", undefined)
      ? { ECHO_PROTOCOL_VERSION: get("protocol-version") }
      : {}),
  },
  transport,
  enabled: true,
  instances:
    backend === "notice"
      ? []
      : [
          {
            instanceId,
            instanceKind:
              backend === "pi" ? "independent-pi" : "independent-echo",
            displayName:
              backend === "pi" ? "Pi coding agent" : "Independent Echo",
            settings: {},
            enabled: true,
          },
        ],
};
if (!validatePluginManifest(manifest))
  throw Error(
    `Invalid package manifest: ${JSON.stringify(validatePluginManifest.errors)}`,
  );
await mkdir(out, { recursive: true });
await cp(path.join(root, "dist/provider.mjs"), path.join(out, "provider.mjs"));
await rm(path.join(out, "contract"), { recursive: true, force: true });
await cp(path.join(root, "contract"), path.join(out, "contract"), {
  recursive: true,
});
await cp(
  path.join(root, "THIRD_PARTY_LICENSES.txt"),
  path.join(out, "THIRD_PARTY_LICENSES.txt"),
);
await writeFile(
  path.join(out, "codepet-provider.json"),
  JSON.stringify(manifest, null, 2),
);
console.log(path.join(out, "codepet-provider.json"));
