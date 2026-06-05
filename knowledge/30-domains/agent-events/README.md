# Agent 事件领域

这个领域覆盖原始 Agent hook payload、本地接入、归一化、标题增强、前端归并和活动过滤。

## 源码模块

- `src-tauri/hooks/code-pet-hook.mjs`：安装到支持的 Agent 配置中的 hook 脚本。
- `src-tauri/src/agent/hooks.rs`：写入托管 hook 项。
- `src-tauri/src/activity/collector.rs`：接收 hook payload。
- `src-tauri/src/activity/events.rs`：将 payload 归一化为 `PetEvent`。
- `frontend/lib/activity.ts`：归并和过滤用户可见活动。

## 测试

- `src-tauri/tests/hook_config_tests.rs`
- `src-tauri/tests/hook_script_tests.rs`
- `src-tauri/tests/event_normalizer_tests.rs`
- `frontend/lib/activity.test.ts`
