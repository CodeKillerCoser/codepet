# Collector JSON UTF-8

## 规则

本地 Collector 返回 JSON 时必须显式声明 `application/json; charset=utf-8`。桌面前端读取近期事件时，只要 Tauri IPC 成功，就必须优先使用 IPC 结果，不再用 localhost HTTP 结果覆盖。

## 适用场景

修改 `src-tauri/src/activity/collector.rs`、`frontend/lib/api.ts`、活动事件轮询、浏览器预览 fallback 或 Windows 桌宠事件标题展示时适用。

## 反例

Collector 只返回裸 `application/json`，Windows 上部分客户端可能按本地代码页或非 UTF-8 路径解码，导致中文标题显示成以 `ä`、`å` 开头的一串乱码。桌面端 IPC 已可直接拿到 Rust/Tauri 的字符串时，再走 HTTP fallback 会扩大这个风险。

## 推荐做法

后端统一通过 JSON 响应 helper 写入 CORS 与 UTF-8 Content-Type。前端 `recentEvents()` 在 `invoke("recent_events")` 成功后直接返回，包括空数组；只有 IPC 不可用或超时时才请求 `http://127.0.0.1:47621/events`。

## 来源

Windows dev 模式下桌宠对话标题出现乱码；原始 Collector 字节按 UTF-8 解码正常，但响应头为 `application/json`，缺少 `charset=utf-8`。

## 验证方式

运行 `npx vitest run frontend/lib/api.test.ts`，确认桌面 IPC 成功时不会调用 HTTP fallback。运行 `cargo test --manifest-path src-tauri/Cargo.toml collector_json_responses_declare_utf8 --lib`，确认 Collector JSON 响应头包含 UTF-8。
