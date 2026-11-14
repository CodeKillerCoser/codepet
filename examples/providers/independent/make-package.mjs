import { mkdir, cp, writeFile, rm } from "node:fs/promises";
import path from "node:path";
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
const out = path.resolve(get("out", "packages/custom"));
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
  JSON.stringify(
    {
      manifestVersion: 2,
      pluginId,
      displayName: backend === "pi" ? "Independent Pi RPC" : "Independent Echo",
      command: get("node", "node"),
      args: ["provider.mjs"],
      env: {
        ECHO_PLUGIN_ID: pluginId,
        ...(backend === "pi" ? { ECHO_BACKEND: "pi" } : {}),
        ...(get("protocol-version", undefined)
          ? { ECHO_PROTOCOL_VERSION: get("protocol-version") }
          : {}),
      },
      transport,
      enabled: true,
      instances: [
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
    },
    null,
    2,
  ),
);
console.log(path.join(out, "codepet-provider.json"));
