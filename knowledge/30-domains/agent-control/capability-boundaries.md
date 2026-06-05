# 能力边界

## 前端契约

`frontend/lib/agentInteractions.ts` 决定卡片操作是否可见。它必须与 `src-tauri/src/agent/actions.rs` 中的后端行为保持一致。

## 当前能力

- 激活：通常可见，但当平台无法按请求的目标类型激活时，后端可以返回不支持。
- Codex 激活：有 session id 时使用 thread deeplink；否则回退到通用 provider target。
- 回复：仅 Codex，且仅限 done/failed，必须有 session id。
- 审批：仅 waiting-approval，通过 collector state 处理。

## 风险

如果前端暴露了后端会拒绝的动作，即使后端技术上是正确的，桌宠窗口也会给用户“坏了”的体验。

## 验证

- 修改 provider capability 时，同步更新前端能力测试和 Rust action 测试。
- 检查 running、waiting approval、done、failed、缺少 session id 和不支持 provider 的场景。
