# 活动归并

## 职责

`frontend/lib/activity.ts` 将近期事件历史转换为桌宠窗口展示的紧凑列表。

## 当前规则

- key 使用 `provider:sessionId`，再回退到 `provider:cwd`，最后回退到 `provider:global`。
- 内部 Codex 后台 prompt 在代码中被过滤。
- 用户自定义标题和消息过滤通过 `ActivityFilterSettings` 应用。
- 隐藏 activity key 会在增量批次之间保留。
- 超过 30 分钟的陈旧 thinking/running activity 会被移除。
- completed 和 failed activity 使用 `endedAt` 或 `createdAt` 作为 footer 时间。

## 为什么重要

大多数可见任务卡片回归来自分组、hidden-key 持久化或终态事件处理。应把这个模块视为共享行为面。

## 验证

- 运行完整 `frontend/lib/activity.test.ts`。
- 修改归并行为后，人工检查桌宠窗口。
