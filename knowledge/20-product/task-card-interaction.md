# 任务卡片交互

## 卡片内容

任务卡片展示标题、消息、provider/source 元数据、状态，以及可用时的终止时间。展示辅助逻辑位于 `frontend/lib/activity.ts`。

## 活动归并

卡片按 provider 加 session id、cwd 或全局 fallback 分组。同一个 activity key 的 active 更新会替换旧卡片。终态事件只有在属于已有可见活动，或能通过 fallback 匹配时才保留。

## 操作

- 打开：尝试激活来源应用或项目路径。
- 移除：从桌宠列表中移除任意任务卡片。
- 移除已完成：清除已完成卡片。
- 回复：只有 provider capability 判断安全时才显示。
- 审批：只在等待审批事件上显示。

## 回复模式

回复模式是 `frontend/PetApp.svelte` 内的本地 UI 状态。它可以打开和关闭，会聚焦 textarea，并让编辑器按内容自适应到最多五行。

## 风险

- 给 footer 增加操作按钮可能压掉底部间距，或造成无意义滚动条。
- 固定任务卡片高度很脆弱，因为回复编辑器、footer 和消息内容都会变化。
- 运行中任务不应显示回复入口，因为后端除已验证控制路径外，无法可靠向 active provider session 注入消息。

## 验证

- 运行 `npx vitest run frontend/lib/activity.test.ts`。
- 修改 `frontend/PetApp.svelte` 时，人工检查单卡、多卡、回复模式和审批模式布局。
