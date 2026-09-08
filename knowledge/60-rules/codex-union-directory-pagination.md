# Codex 并集发现与分页快照边界

## 规则

无 cursor 的会话列表请求重新拉取来源，不复用上次首页；带 cursor 的请求只能读取其绑定的固定摘要快照。App Server cursor、DB 扫描边界与客户端 cursor 不得混用。

## 适用场景

Codex Provider 普通、项目、standalone、时间与 ID 列表，以及新增来源或修改 SnapshotPager 调用的变更。

## 反例

- 原生第一页和 DB 第一页取并集后立即返回，误称为全局排序第一页。
- 冻结 ID，却在后续页补读后重新过滤或排序。
- 无条件让 legacy read 的创建时间覆盖列表/DB 的较新更新时间。
- 仅因本轮 list 没出现或读取失败就认定任务已删除。
- 接到实时事件便使全部历史 cursor 失效，或者过期时静默从第一页重来。
- 为修复分页删除 0.151/0.152 项目映射、unknown-method 或 history 兼容分支。

## 推荐做法

先收集候选超集，补读缺失对象，按字段和来源合并，再项目装饰、过滤、稳定排序并固定摘要。用唯一 ID 兜底排序，生命周期 generation 与查询条件绑定 cursor；事件 ID 仅补充发现，不能伪造对象或全局活动状态。

DB 只读短查询，读取失败与不存在分开；不在网络 RPC 期间持有 DB 事务。补读并发有界，未知失败显式传播。保留原生项目优先和未映射项目非 standalone 规则；源方法明确不支持的兼容降级不能扩展成忽略全部错误。

## 来源

[双源目录分页修复](../40-runbooks/codex-union-directory-pagination.md)，以及用户关于并集、无首页缓存、cursor 和 0.151/0.152 兼容的明确要求。

## 验证方式

运行 Codex Provider 的 `union_directory` 集成用例、`directory::tests`，以及 SDK `conversation_query` 测试；版本兼容用例必须覆盖 0.151 和 0.152。检查新首页可见更新、旧 cursor 固定成员和字段、项目/时间边界、不完整扫描错误、事件独有 ID、DB 锁/WAL 与生命周期隔离。
