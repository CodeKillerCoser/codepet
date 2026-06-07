import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";
import { realpathSync } from "node:fs";
import { resolve } from "node:path";

const workspaceRoot = process.cwd();
const buildRoot = realpathSync(workspaceRoot);
const devFsAllow = Array.from(new Set([workspaceRoot, buildRoot]));

export default defineConfig(({ command }) => {
  const projectRoot = command === "build" ? buildRoot : workspaceRoot;

  return {
    root: projectRoot,
    plugins: [svelte()],
    clearScreen: false,
    server: {
      port: 1420,
      strictPort: true,
      fs: {
        allow: devFsAllow,
      },
    },
    build: {
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
