# Hook 不生效

## 现象

某个已启用的 Agent 没有在桌宠窗口中产生任务卡片。

## 需要收集的证据

- `git status --short`，区分本地改动和运行时状态。
- App UI 中的 Agent 启用状态，或 `list_agents` 行为。
- 生成后的 Agent 配置文件，尤其是托管命令。
- `code-pet-hook.mjs` 是否存在于 local app data 目录。
- 应用运行时 `http://127.0.0.1:47621/health` 的 collector 健康状态。
- `code-pet/logs/code-pet.log` 中的 collector 或启动错误。

## 排查步骤

1. 根据 `src-tauri/src/agent/registry.rs` 确认 Agent 配置路径。
2. 确认托管项包含所有预期 hook 事件。
3. 确认 Windows 或 Unix 命令转义没有破坏命令。
4. 触发一个小 Agent 任务，检查 `/hook` 是否收到数据。
5. 如果任务执行时应用未启动，检查 `~/.code-pet/spool/events.jsonl`。

## 修复后验证

- `cargo test --manifest-path src-tauri/Cargo.toml hook_config_tests hook_script_tests`
- 从受影响 Agent 人工触发一个任务。
