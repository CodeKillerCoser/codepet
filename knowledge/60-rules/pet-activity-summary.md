# 桌宠活动摘要保留执行中回退

## 规则

PetTask.summary 表示活动摘要，不限于助手回复。不能因工具 hook 缺少 last_assistant_message 而移除已有的进展内容来源。

## 适用场景

Host pet_gateway/projection.rs 的共享任务投影，以及消费 summary 的顶部和桌面视图。

## 反例

只读取 last_assistant_message，导致持续收到工具事件的执行中任务始终显示“尚未收到回复内容”；新轮继承旧回复，或迟到的旧轮 Stop 覆盖当前摘要。

## 推荐做法

恢复既有优先级：助手回复、prompt、error、原生 error.data.message、工具 description/command/file_path。忽略空白字符串，保留既有长度上限。UserPromptSubmit 先清空旧摘要且不读取其助手回复字段；没有新内容的事件保留本轮摘要。复用时间和 turn 隔离，不把整个工具响应序列化为摘要。

## 来源

[2026-11-10 排查与修复](../40-runbooks/hook-not-working.md)：ea9ee1e 移除了执行中回退；用户要求恢复。

## 验证方式

运行 Host pet_gateway::projection::tests，覆盖无回复工具事件、描述/命令/路径、空字段、回复优先、跨轮和迟到事件，以及共享投影的 OpenCode 错误摘要。安装后验证真实 Codex 任务在执行时更新活动摘要、结束时显示回复。
