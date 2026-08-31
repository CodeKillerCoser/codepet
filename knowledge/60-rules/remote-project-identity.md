# Remote 项目身份归并规约

## 规则

Remote 项目投影不得直接把 Codex `thread.cwd` 当作项目身份。现存 Git workspace 必须从 `.git`/`commondir` 和 remote URL 提取仓库身份；同一 common-dir 的主 checkout 与 linked worktree 共享项目，同一 remote 的不同 clone 也归入同一逻辑项目。普通非 Git workspace 以规范化绝对路径为身份。

`<CODEX_HOME>/worktrees/<id>/<project-name>`、`~/.codex/worktrees/<id>/<project-name>` 及从 cwd 明确识别出的 Codex managed worktree，即使目录已经删除，也必须形成稳定的逻辑项目键。批次或当前 managed 根只有一个同名仓库身份时使用其代表 root；没有仓库证据时使用 `managed-root/project-name` 兜底。若已经证明存在多个同名但不同的仓库身份，无法归属的历史 cwd 保留原路径，不能误并。项目归并只改变 workspace/project projection，不得改写 native conversation id、四段 `RoutedResourceId`、事件通道或历史消息。

## 适用场景

- 修改 Codex Provider 的 `conversation.list/get/create` 或 `thread/started` 会话投影。
- Remote Client 按 `workspaceRoot` 构造项目 name、root 或稳定 UI key。
- 处理 Codex worktree、已删除历史 worktree、同 remote clone、符号路径或同名仓库。

## 反例

- 按 cwd 字符串精确分组：每个 `worktrees/<id>/codepet` 都会显示为一个同名项目。
- 按目录 basename 合并：两个互不相关但同名的仓库会被误并。
- 所有失效 worktree 都保留原路径：历史会话会继续制造大量重复项目。
- 使用 `Thread.projectId` 作为唯一依据：实机 `conversation.list` 样本中该字段可能全部为 `null`。
- 为项目归并修改 conversation/resource identity：会破坏 Gateway 路由、详情读取与事件关联。

## 推荐做法

- 先规范化绝对 workspace path；路径存在时沿祖先查找 `.git`。
- linked worktree 从 `.git` 文件解析 git-dir，再从 `commondir` 得到共享仓库身份；常规非 bare 仓库以 common-dir 的父目录作为稳定项目 root。
- 从 Git config 优先读取 `origin`，只有一个 remote 时读取该 remote；相同规范化 remote 的 clone 选择确定性的代表 root。没有 remote 时使用 common-dir 区分仓库。
- remote URL 只作为进程内比较证据，不写入 `workspaceRoot`、Provider extension 或日志。
- `conversation.list` 必须按返回批次准备投影证据，并扫描该批次涉及的 managed worktree 根；这样分页中只出现已删除 cwd 时，也能利用当前仍存在的同名 worktree 找回仓库身份。
- 对失效 managed cwd 使用有限的 basename fallback：无身份反证时按 `managed-root/project-name` 归并；发现多个仓库身份时保守保留无法归属的原路径。这是为“历史项目只显示一次”接受的产品权衡，不适用于普通目录。
- 保留原始 cwd 到 Provider extension 的 `nativeCwd`，只把逻辑项目 root 写入 `workspaceRoot`。
- basename fallback 生成的逻辑 root 可能不存在，不能作为文件系统执行目录；会话操作仍通过原生 resource identity 路由，原始目录由 `nativeCwd` 保留。
- 项目索引保持为可重建投影，不新增通用项目数据库。

## 涉及模块

- `workspace_projection.rs`：读取当前批次、managed worktree 根和 Git metadata，按 remote/common-dir 建立临时证据并输出确定性项目 root，不保存跨请求项目索引。
- `protocol.rs` 与 `client.rs`：`thread/list` 先为整个返回批次准备投影，其余 snapshot 使用相同单条入口；文件 I/O 位于 App Server 工作线程，不进入 mapper 锁或异步 RPC dispatcher。
- `mapper.rs`：继续使用 snapshot 中已投影的 `workspaceRoot`，同时从原始 thread 保留 `nativeCwd` 和 resource identity。

## 来源

2026-08-31 实机复现：CodePet Remote 首屏 50 条 `conversation.list` 会话产生 49 个原始 workspace 项目，其中 47 条来自不同的 Codex `codepet` managed worktree；样本 `projectId` 全为 `null`。Provider 原样透传 cwd，Remote 再按完整路径分组，是重复项目的直接原因。仅依赖现存 common-dir 时只能得到 37 个项目，因为 34 个历史 worktree 已删除；加入受限 managed basename fallback 后得到 3 个逻辑组，会话数为 47/2/1。`codepet` 的 47 条归为 1 组；`codepet-remote` 保持 2 组，因为匿名文件系统证据分别是“现存 Git root，2 条”和“现存非 Git root，1 条”，不是同一仓库的 worktree。长期边界见 `knowledge/10-architecture/runtime-gateway-and-remote-control.md` 的“项目和普通聊天”。Bug 的引入提交未确认。

## 验证方式

- 单测覆盖主 checkout/linked worktree、separate git-dir、非 Git 路径、同 remote clone、同名不同仓库、全删除 managed fallback，以及存在一个或多个仓库证据时的失效 cwd 行为。
- wire 测试必须覆盖同一个 `thread/list` 返回中的现存与已删除 managed worktree，防止批次准备接线丢失。
- mapper 测试同时断言规范化后的 `workspaceRoot`、原始 `nativeCwd` 和完整四段 resource identity。
- 用真实 Codex Provider 重跑首屏 `conversation.list`，只记录会话数、原始/投影项目数、匿名分组依据与 route mismatch 数，不输出消息正文、origin 或完整路径。
- review 确认 Gateway/Provider/历史消息 Schema 未变化，并运行 Provider 全目标测试与 `git diff --check`。
