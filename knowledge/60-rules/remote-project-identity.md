# Remote 项目身份归并规约

## 规则

Remote 项目投影不得直接把 Codex `thread.cwd` 当作项目身份。现存 Git workspace 必须从 `.git`/`commondir` 解析仓库身份，并以主 checkout 的规范化绝对路径作为项目 root；同一 common-dir 的主 checkout 与 linked worktree 共享项目。非 Git workspace 以规范化绝对路径为身份，同名但 common-dir 不同的仓库不得合并。

已删除、损坏或无法读取 Git 元数据的 workspace 必须保留规范化原路径，不能用 origin、目录名或当前可见候选猜测 common-dir。项目归并只改变 workspace/project projection，不得改写 native conversation id、四段 `RoutedResourceId`、事件通道或历史消息。

## 适用场景

- 修改 Codex Provider 的 `conversation.list/get/create` 或 `thread/started` 会话投影。
- Remote Client 按 `workspaceRoot` 构造项目 name、root 或稳定 UI key。
- 处理 Codex worktree、已删除历史 worktree、符号路径或同名仓库。

## 反例

- 按 cwd 字符串精确分组：每个 `worktrees/<id>/codepet` 都会显示为一个同名项目。
- 按目录 basename 合并：两个互不相关但同名的仓库会被误并。
- 使用 `Thread.projectId` 作为唯一依据：实机 `conversation.list` 样本中该字段可能全部为 `null`。
- 为项目归并修改 conversation/resource identity：会破坏 Gateway 路由、详情读取与事件关联。

## 推荐做法

- 先规范化绝对 workspace path；路径存在时沿祖先查找 `.git`。
- linked worktree 从 `.git` 文件解析 git-dir，再从 `commondir` 得到共享仓库身份；常规非 bare 仓库以 common-dir 的父目录作为稳定项目 root。
- 保留原始 cwd 到 Provider extension 的 `nativeCwd`，只把逻辑项目 root 写入 `workspaceRoot`。
- 已删除或不可验证的 worktree fail-soft 到规范化原路径；若产品未来要归并这类历史路径，必须先获得能精确关联 common-dir 的权威元数据。
- 项目索引保持为可重建投影，不新增通用项目数据库。

## 涉及模块

- `workspace_projection.rs`：只读取当前 workspace 的路径与 Git metadata，输出稳定项目 root，不保存跨请求项目索引。
- `protocol.rs` 与 `client.rs`：在 App Server 工作线程中创建 snapshot 时完成投影，避免在 mapper 锁和异步 RPC dispatcher 中做同步文件 I/O。
- `mapper.rs`：继续使用 snapshot 中已投影的 `workspaceRoot`，同时从原始 thread 保留 `nativeCwd` 和 resource identity。

## 来源

2026-08-31 实机复现：CodePet Remote 首屏 50 条 `conversation.list` 会话产生 49 个原始 workspace 项目，其中 47 条来自不同的 Codex `codepet` worktree；样本 `projectId` 全为 `null`。Provider 原样透传 cwd，Remote 再按完整路径分组，是重复项目的直接原因。13 个仍保留 Git metadata 的 linked worktree 可安全归为 1 个项目；34 个已删除 workspace 无法证明 common-dir，必须保留为独立规范化路径，因此保守投影为 37 个项目。长期边界见 `knowledge/10-architecture/runtime-gateway-and-remote-control.md` 的“项目和普通聊天”。Bug 的引入提交未确认。

## 验证方式

- 单测覆盖主 checkout/linked worktree、separate git-dir、非 Git 路径、同名不同仓库和已删除 workspace 的保守回退。
- mapper 测试同时断言规范化后的 `workspaceRoot`、原始 `nativeCwd` 和完整四段 resource identity。
- 用真实 Codex Provider 重跑首屏 `conversation.list`，只记录会话数、原始/投影项目数、匿名分组依据与 route mismatch 数，不输出消息正文、origin 或完整路径。
- review 确认 Gateway/Provider/历史消息 Schema 未变化，并运行 Provider 全目标测试与 `git diff --check`。
