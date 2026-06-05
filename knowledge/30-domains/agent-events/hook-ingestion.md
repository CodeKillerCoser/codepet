# Hook 接入

## 当前流程

`src-tauri/src/agent/hooks.rs` 为每个支持的 Agent hook 事件安装托管 Node 命令。命令会以 `--agent <id>` 运行 `code-pet-hook.mjs`，可用时还会带上 `--event <event>`。

Hook 脚本会把 payload 发送到 `http://127.0.0.1:47621/hook`。如果应用不可用，事件可暂存到 `~/.code-pet/spool/events.jsonl`，并在启动时回放。

## Provider 配置

Agent 注册数据位于 `src-tauri/src/agent/registry.rs`。README 记录了 Codex、Claude Code、Qoder 和 Cursor 当前使用的配置文件。

## Windows 风险

Windows 命令转义具有平台差异。`hooks.rs` 在 Windows 实现的 `command_arg_quote()` 中转义 `%` 和引号。如果 Windows hook 执行失败，先检查生成配置里的命令。

## 验证

- `cargo test --manifest-path src-tauri/Cargo.toml hook_config_tests`
- `cargo test --manifest-path src-tauri/Cargo.toml hook_script_tests`
