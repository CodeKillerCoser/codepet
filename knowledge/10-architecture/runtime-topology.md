# 运行拓扑

## 组件

- 主窗口：`frontend/App.svelte`，由 `index.html` 加载。
- 桌宠悬浮窗：`frontend/PetApp.svelte`，由 `pet.html` 加载。
- Tauri 后端：`src-tauri/src/lib.rs` 注册命令、托盘行为、插件、启动工作和窗口。
- 本地 collector：`src-tauri/src/activity/collector.rs` 在 `127.0.0.1:47621` 暴露 HTTP 路由。
- Hook 脚本：`src-tauri/hooks/code-pet-hook.mjs` 由 `src-tauri/src/agent/hooks.rs` 安装到本地 app data 目录。

## 流程

1. 用户在主窗口启用某个 Agent。
2. Rust 将托管 hook 项写入该 Agent 配置。
3. Agent 调用 `code-pet-hook.mjs`。
4. 脚本把 payload 发送到 `/hook`。
5. Rust 归一化并保存 `PetEvent`。
6. Rust 向桌宠窗口发出 `pet-event`。
7. 桌宠窗口把事件归并成任务卡片，并在设置允许时播放声音。

## 验证

- 前端行为由 `frontend/lib/activity.test.ts`、`frontend/lib/sound.test.ts` 和组件测试覆盖。
- 后端 collector 与 hook 行为由 `src-tauri/tests/` 下的测试覆盖。
