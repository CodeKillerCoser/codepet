# Codex Thread Title Source

## 规则

Codex 任务卡片的可见标题应优先使用 `.codex/session_index.jsonl` 中对应 session id 的 `thread_name`。Codex 的 assistant 回复摘要只能作为正文或详情，不能作为标题 fallback。

## 适用场景

修改 `src-tauri/src/activity/title_resolver.rs`、`src-tauri/src/agent/codex_audit.rs`、`frontend/lib/activity.ts`、Codex hook payload 解析、audit replay 或桌宠卡片标题逻辑时适用。

## 反例

实时 hook 事件不解析 Codex session_index，终态事件又把 `last_assistant_message` 当作 generic title 的 fallback，会导致桌宠显示某条回复摘要，而不是 Codex 对话列表里的标题。

## 推荐做法

后端实时 Collector 和 Codex audit replay 都要能从 session id 解析 `thread_name`。前端只有 active prompt 这类用户输入可以作为临时标题；`done` 和 `failed` 等终态事件不应把 assistant summary 提升成标题。

## 来源

Windows dev 模式下，Collector 事件的 session id 可以在 `.codex/session_index.jsonl` 中查到 `thread_name`，但实时 hook 事件仍使用 generic title，前端因此显示了 Codex 回复摘要。

## 验证方式

运行 `cargo test --manifest-path src-tauri/Cargo.toml --test title_resolver_tests`、`cargo test --manifest-path src-tauri/Cargo.toml --test codex_audit_tests` 和 `npx vitest run frontend/lib/activity.test.ts`。
