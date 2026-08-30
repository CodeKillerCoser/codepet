# 运行拓扑

## 组件

- 主窗口：`frontend/App.svelte`，由 `index.html` 加载。
- 桌宠悬浮窗：`frontend/PetApp.svelte`，由 `pet.html` 加载。
- Tauri 后端：`src-tauri/src/lib.rs` 注册命令、托盘行为、插件、启动工作和窗口。
- 本地 collector：`src-tauri/src/activity/collector.rs` 在 `127.0.0.1:47621` 暴露 HTTP 路由。
- Hook 脚本：`src-tauri/hooks/code-pet-hook.mjs` 由 `src-tauri/src/agent/hooks.rs` 安装到本地 app data 目录。
- Codex Remote：`ProviderHostState` / `PluginManager`、独立 `codepet-provider-codex` 进程、`ProviderGatewayService`、compat `RuntimeGatewayState` 和 `runtime-gateway-event`。
- Codex Desktop Companion：`CodexDesktopCompanionState`、`~/.codex/ipc/ipc.sock` adapter 和 `codex-desktop-companion-event`。

旧 Hook 拓扑当前只服务 Claude Code、Qoder 和 Cursor。Codex 在设置页保留无 Hook 的 disabled 占位项；启动时会清理遗留托管 Codex Hook，脚本、实时 collector 和 spool 回放也都会拒绝 Codex。Codex audit watcher 与启动回放已移除。历史 Token 用量扫描仍是独立路径，本阶段不变。

## Codex 双链路

```text
远程控制请求
  → runtime_gateway_request
  → compat 薄适配 / ProviderGatewayService
  → PluginManager / codepet-provider-codex
  → 官方 Codex App Server stdio JSON-RPC

Codex Desktop Owner/Follower 状态
  → ~/.codex/ipc/ipc.sock
  → companion adapter / Gateway
  → codex_desktop_companion_snapshot/replay/event
  → PetApp activity store
```

两路各自拥有 registry、event bus、sequence、replay 和 unavailable 状态，且不共享 thread provenance 或投影状态。App Server 只在 Provider instance start 中初始化且有超时；失败不阻塞 companion。桌宠不订阅 `runtime-gateway-event`，remote conversation/turn/approval 不能进入 activity store。remote 与 Desktop 即使报告相同 native thread id，也分别留在各自通道；本阶段不做跨链路排除、去重或同步。

App Server 自动让 Desktop 加载 thread 是产品协同，不改变双链路边界。Hook、audit、transcript 和文件监听不参与 Codex 数据。

## 旧 Hook 流程

1. 用户在主窗口启用 Claude Code、Qoder 或 Cursor。
2. Rust 将托管 hook 项写入该 Agent 配置。
3. Agent 调用 `code-pet-hook.mjs`。
4. 脚本把 payload 发送到 `/hook`。
5. Rust 归一化并保存 `PetEvent`。
6. Rust 向桌宠窗口发出 `pet-event`。
7. 桌宠窗口把事件归并成任务卡片，并在设置允许时播放声音。

## 验证

- 前端行为由 `frontend/lib/activity.test.ts`、`frontend/lib/sound.test.ts` 和组件测试覆盖。
- 后端 collector 与 hook 行为由 `src-tauri/tests/` 下的测试覆盖。
