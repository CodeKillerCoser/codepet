# 条件渲染元素聚焦

## 规则

当需要聚焦一个条件渲染出来的元素时，必须等 DOM 已经挂载该元素后，再调用 focus 或设置 selection。

## 适用场景

- Svelte `{#if}` 块挂载 input、textarea、button 或 dialog。
- 点击处理会切换状态，并立刻尝试聚焦新渲染的元素。
- 列表中复用同一个元素 binding。

## 反例

点击回复按钮设置 `replyingToId`，但 focus 在 textarea 出现前执行。用户看到回复模式，却不能立刻输入。

## 推荐做法

在 Svelte 的 DOM 更新边界之后再 focus。当目标项消失或失去 capability 时，保持清晰的重置路径。

## 来源

桌宠任务卡片的回复编辑器焦点回归。

## 验证方式

修改条件输入时，执行人工 click-to-reply 检查，并 review `frontend/PetApp.svelte`。
