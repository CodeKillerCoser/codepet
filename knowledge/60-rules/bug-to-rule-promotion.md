# Bug 到规约的升级

## 规则

当 Bug 暴露出可复用约束时，更新相关 runbook 或新增规约。不要把经验只留在聊天记录里。

## 适用场景

- 同类 Bug 已经发生不止一次。
- 修复触及共享 UI 状态、provider capability、窗口行为或设置持久化。
- review 发现未来 Agent 很可能重复踩到的回归风险。

## 反例

因为局部 UI 便利而移动回复按钮，但没有记录 provider capability 边界。后续 Agent 又在运行中任务上暴露同一个按钮，导致流程再次出错。

## 推荐做法

写一条短规约，包含来源、反例和验证方式。通过路径链接到相关领域文档或 runbook。

## 来源

项目关于“让活文档随每次变更和 Bug 修复保持一致”的讨论。

## 验证方式

Bug 修复后，检查是否需要更新 `knowledge/40-runbooks/`、`knowledge/50-decisions/` 或 `knowledge/60-rules/`。
