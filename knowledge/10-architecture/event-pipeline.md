# 事件管线

## 来源

Agent hook payload 由 `src-tauri/hooks/code-pet-hook.mjs` 接收，并发送到本地 collector：`http://127.0.0.1:47621/hook`。

## 归一化

`src-tauri/src/activity/events.rs` 将原始 payload 转换为 `PetEvent`：

- provider、kind、status、title、message、session id、cwd、tool name、source metadata 和 ring 标记。
- Cursor 事件名会被规范化为共享事件词表。
- 空闲通知里的模板化噪声会被抑制。
- 失败信号可以把终态通知转换为失败任务事件。

## 增强与存储

`src-tauri/src/activity/title_resolver.rs` 改善任务标题。`src-tauri/src/app/state.rs` 保存近期事件、限制前端输出数量，并跟踪待审批状态。

## 前端归并

`frontend/lib/activity.ts` 按 provider 加 session 或 cwd 分组，过滤内部/后台事件，隐藏用户配置命中的过滤项，丢弃过期 active work，并只保留属于可见活动的终态卡片。

## 风险

- 修改事件 identity 可能把无关任务合并，或把一个任务拆成多个卡片。
- 修改终态事件处理可能重新引入孤立完成卡片。
- 过滤逻辑必须在增量批次中保留隐藏 key，否则已过滤的后台任务可能重新出现。

## 验证

- 运行 `npx vitest run frontend/lib/activity.test.ts`。
- 运行 `src-tauri/tests/event_normalizer_tests.rs` 下相关 Rust 归一化测试。
