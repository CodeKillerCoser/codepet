# 事件归一化

## 职责

`src-tauri/src/activity/events.rs` 将不同 provider 的 payload 形态转换为共享 `PetEvent` 模型。

## 受保护行为

- Cursor 事件名会映射到共享生命周期名称。
- 空闲通知会作为 completed/idle 终态处理，并清空模板化消息。
- 终态样式事件中的失败信号可以生成 failed task。
- `code_pet` 或 `codePet` 下的 source metadata 会被保留，用于终态/app 来源展示和激活。

## 风险

- Provider payload 不稳定；新增字段应保持 additive。
- 标题或消息提取变化会影响过滤和卡片展示。
- `sessionId` 和 `cwd` 会影响活动归并。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml event_normalizer_tests`。
- 如果归一化字段影响卡片分组或标签，运行 `npx vitest run frontend/lib/activity.test.ts`。
