import { execFileSync } from "node:child_process";
import { build } from "esbuild";
import { mkdir, cp, writeFile, rm } from "node:fs/promises";
await build({
  entryPoints: ["provider.mjs"],
  bundle: true,
  platform: "node",
  target: "node24",
  format: "esm",
  outfile: "dist/provider.mjs",
});
for (const [suffix, transport] of [
  ["jsonl", "stdio-jsonrpc-v1"],
  ["zstd", "stdio-codepet-zstd-v1"],
  ["mux", "stdio-codepet-mux-v1"],
]) {
  const dir = `packages/${suffix}`;
  await rm(dir, { recursive: true, force: true });
  await mkdir(dir, { recursive: true });
  await cp("dist/provider.mjs", `${dir}/provider.mjs`);
  await cp("contract", `${dir}/contract`, { recursive: true });
  await cp("THIRD_PARTY_LICENSES.txt", `${dir}/THIRD_PARTY_LICENSES.txt`);
  await writeFile(
    `${dir}/codepet-provider.json`,
    JSON.stringify(
      {
        manifestVersion: 2,
        pluginId: `independent.echo.${suffix}`,
        displayName: `Independent Echo (${suffix})`,
        command: "node",
        args: ["provider.mjs"],
        env: { ECHO_PLUGIN_ID: `independent.echo.${suffix}` },
        transport,
        enabled: true,
        instances: [
          {
            instanceId: `independent-echo-${suffix}`,
            instanceKind: "independent-echo",
            displayName: "Independent Echo",
            settings: {},
            enabled: true,
          },
        ],
      },
      null,
      2,
    ),
  );
}

execFileSync(
  process.execPath,
  [
    "make-package.mjs",
    "--out",
    "packages/pi-jsonl",
    "--plugin-id",
    "independent.pi.jsonl",
    "--instance-id",
    "independent-pi-jsonl",
    "--backend",
    "pi",
  ],
  { stdio: "inherit" },
);
