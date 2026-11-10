# 任务卡片交互

## 当前数据与展示

桌宠只消费 Pet Gateway 的 `PetTask`。后端 `pet_gateway/projection.rs` 负责来源事件到任务的投影；前端 `pet/model.ts` 负责最新排序、关闭历史和连接错误。原 `activity.ts` 的 `PetEvent` 归并已删除。

桌面模式使用 `PetActivityGroups.svelte`，按等待交互、进行中、已完成分组：waiting-approval/waiting-input/unknown/failed 属于等待交互，running 属于进行中，completed/interrupted 属于已完成。每组可展开，组状态存于宿主，模式切换保留。顶部模式使用 `pet/NotchPet.svelte` 的横向任务列表，固定顶栏，列表单独滚动。

标题、Agent 名称、实际状态与最近更新时间来自任务字段；时间为 updatedAt，不是持续运行时长。Markdown 经过统一净化；桌面消息保持一行，顶部最多两行，链接仍可打开。顶部缺少正文时明确提示“尚未收到回复内容”。

`summary` 是活动摘要：有助手回复时优先显示回复，否则可显示提示词、错误或工具描述、命令、文件路径。工具 hooks 无回复字段时仍能更新进展；新轮清空旧摘要并忽略上轮回复，空事件保留本轮摘要。参见 [活动摘要规约](../60-rules/pet-activity-summary.md)。

## 操作与边界

支持隐藏任务、桌面清除已完成任务、模式切换、打开主窗口、角色互动。隐藏记录在该任务有更新前有效，两套 UI 共用。当前不提供来源会话激活、回复、审批或打断按钮；后端 Agent 和远程控制能力不等于桌宠已接入控制协议。

## 风险与验证

新增动作需检查协议、键盘焦点、窄屏可见性和原生透传。隐藏堆叠卡不得拦截鼠标；模式切换不可产生第二条任务订阅。运行 `frontend/lib/pet/model.test.ts`、`activityGroups.test.ts` 及顶部/桌面分组浏览器 QA，原生拖动和透传另在目标系统验证。
