# Vite worktree 路径口径约束

## 规则

Vite 多 HTML 入口配置必须让 `root` 和 `rollupOptions.input` 在同一个命令内使用同一种路径口径。Windows Codex worktree 可能同时存在展示路径 `C:\Users\...\codepet` 和真实路径 `E:\.codex\...\codepet`：`build` 应统一使用 realpath；`dev` 应保留 `process.cwd()` 作为 root，并把展示路径和 realpath 都加入 `server.fs.allow`。

## 适用场景

修改 `vite.config.ts`、新增 HTML entry、调整 build/dev server 路径，或在 Windows worktree 中排查 `npm run build`、`npm run dev`、`npm run tauri dev` 启动失败时适用。

## 反例

- `root` 隐式使用当前展示路径，但 HTML input 使用 `__dirname` 的真实路径。Rollup 可能把 `E:/.codex/.../index.html` 当成输出文件名，报 `fileName or name properties ... must be strings that are neither absolute nor relative paths`。
- dev server 直接把 `root` 设置为 `realpathSync(process.cwd())`，但 Vite dep optimizer 内部仍用 `process.cwd()` 计算 esbuild 输出相对路径。两者跨盘符时可能在 ready 后崩溃：`TypeError: Cannot read properties of undefined (reading 'imports')`。
- dev server 使用展示路径 root，但未把 realpath 加入 `server.fs.allow`。Svelte/Vite 可能把模块解析到真实路径，导致 pre-transform 报文件不存在。

## 推荐做法

在 `vite.config.ts` 中同时计算 `workspaceRoot = process.cwd()` 和 `buildRoot = realpathSync(workspaceRoot)`：

- `command === "build"` 时使用 `buildRoot` 作为 root，并用同一个 root 解析 `index.html` 和 `pet.html`。
- dev 时使用 `workspaceRoot` 作为 root，避免 Vite dep optimizer 跨盘符计算 cache 输出。
- dev server 的 `server.fs.allow` 包含 `workspaceRoot` 和 `buildRoot`。

## 来源

Windows Codex worktree 中先后出现两个路径口径问题：`npm run build` 因跨盘符绝对 HTML 输出名失败；`npm run dev` 在 Vite ready 后因 dep optimizer 读取 `output.imports` 崩溃。按命令区分 root，并允许两种路径后，build 和 dev 都通过。

## 验证方式

- `npm run build`，确认两个 HTML 入口都能生成到 `dist/`。
- `npm run dev` 保持运行 30 秒以上，确认不会在 Vite ready 后崩溃。
- 请求 `http://127.0.0.1:1420/`、`/frontend/main.ts`、`/frontend/pet.ts` 均应返回 200，stderr 不应出现 pre-transform 路径错误。
