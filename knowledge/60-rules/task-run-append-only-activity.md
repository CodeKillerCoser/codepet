# TaskRun 活动只能追加

## 规则

TaskRun 的既有 timeline 是不可变前缀。所有成功写入必须追加可追溯的活动；不能用新评论替换状态、计划、验证等变更事件，也不能编辑、删除、重排已有活动。

## 适用场景

Task/Run 的领域写入、MCP 协作评论、App 元数据和状态操作、迁移及后续 writer 升级。读取和失败事务不追加活动，幂等重试复用原回执。

## 反例

有 note 就只保存 note，导致同次计划/状态变化丢失；为了“整理评论”修改旧 summary；旧 writer 反序列化后丢弃新事件字段；拿 Run 上一次工作区当成跨 Agent 评论者的 Git 观测。

## 推荐做法

事务内读取最新聚合，保留旧前缀，按事实追加评论与各类业务事件，保存字段 before/after 和实际连接身份。评论不要求内容 revision，业务更新仍使用 expectedRevision；回复目标必须属于同一 Run。历史/已完成 Run 允许追加评论，不因此修改开发内容或解除人工锁。

提交前校验旧事件等值、新 ID 唯一。新元数据不能被旧 writer 保真保存时，提升记录 schema 并拒绝旧写入；沿用原稳定目录和只读迁移，不重写历史来源。修正通过新事件引用旧事件表达。

## 来源

2026-11-26 用户要求跨 workspace、多 Agent 交互与 append-only 活动线；实现和证据见 [TaskRun 活动说明](../30-domains/task-management/task-run-activity.md)。

## 验证方式

运行 tasks/activity_tests.rs、MCP HTTP/stdio/managed、TaskRunTimeline.test.ts。必须覆盖组合更新中的评论与事件、并发和重试、历史 Run 回执、篡改回滚、旧 schema、游标边界；UI 检查回复跳转、实时追加、编辑草稿、焦点及窄屏。
