import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";
import { realpathSync } from "node:fs";
import { resolve } from "node:path";
import { version } from "./package.json";

const workspaceRoot = process.cwd();
const buildRoot = realpathSync(workspaceRoot);
const devFsAllow = Array.from(new Set([workspaceRoot, buildRoot]));

export default defineConfig(({ command }) => {
  const projectRoot = command === "build" ? buildRoot : workspaceRoot;
  const builtAt = Date.now();

  return {
    root: projectRoot,
    define: { __WEB_BUILD_AT__: JSON.stringify(builtAt) },
    plugins: [svelte(), {
      name: "webcontent-build-info",
      generateBundle() {
        this.emitFile({ type: "asset", fileName: "build-info.json", source: JSON.stringify({ version, builtAt }) });
      },
    }],
    clearScreen: false,
    server: {
      port: 1420,
      strictPort: true,
      fs: {
        allow: devFsAllow,
      },
    },
    build: {
      outDir: "webcontent",
      target: "es2022",
      rollupOptions: {
        input: {
          main: resolve(projectRoot, "index.html"),
          pet: resolve(projectRoot, "pet.html"),
        },
      },
    },
  };
});
