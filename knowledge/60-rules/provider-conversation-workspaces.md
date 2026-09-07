# Provider 新建会话的项目与工作目录

## 规则

项目归属和执行目录分别传递。CodePet 管理的目录位于 Host 用户的 `~/.codepet/remote_workspace/<provider>/`，非项目任务使用 `task/<任务ID>/`，工作树使用 `worktree/<工作树ID>/`。

平台展开后的默认根目录为：

| 平台 | Code Pet 管理的 workspace 根 |
| --- | --- |
| Windows | `%USERPROFILE%/.codepet/remote_workspace/<provider>/`，通常即 `C:/Users/<用户>/.codepet/remote_workspace/<provider>/` |
| macOS | `/Users/<用户>/.codepet/remote_workspace/<provider>/` |

实现必须通过平台 home 目录 API 和 `Path` / `PathBuf` 逐段构造，表中的分隔符只用于展示。该根目录不属于 Code Pet 应用数据目录，也不属于 `CODEX_HOME`、`CLAUDE_CONFIG_DIR` 或 OpenCode XDG 数据目录；修改任一数据目录都不得隐式移动、删除或重写 workspace。

## 适用场景

Remote 新建会话、Provider 的 conversation.create 与 Codex thread/start 参数转换。defaultWorkspaceRoot 保持 Provider 层，不能改成 task 层后又由 Remote 重复拼接。

## 反例

只传 projectId 会丢失执行目录，App Server 不保证按项目根目录启动。把 CodePet 工作树放进 CODEX_HOME/worktrees 会混入 Codex 桌面端管理范围。用工作树目录反推项目归属会使会话脱离原项目。

## 推荐做法

Remote 从所选项目取根目录，连同 routed project 和模式提交。main 直接使用该目录；worktree 由 Codex Provider 创建独立 detached 工作树，再以新目录调用 thread/start，同时保留 projectId。项目子目录要保留相对路径。非项目目录根据 Provider 广告的根路径分配，其他 Provider 仅广告实际支持的模式。新规则只作用于新建目录，不迁移已有会话。

目录分层固定为：

```text
~/.codepet/remote_workspace/
├── codex/
│   ├── task/<任务ID>/
│   └── worktree/<工作树ID>/
├── claude/
│   └── task/<任务ID>/
└── opencode/
    └── task/<任务ID>/
```

- 无项目会话由 Remote 在 Provider 广告的根下分配 `task/<任务ID>/`，Provider 创建实际目录。
- 项目 `main` 模式直接使用用户选择的项目目录，可以位于任意磁盘，不复制到 `.codepet`。
- Codex `worktree` 模式在 `codex/worktree/<工作树ID>/` 创建 detached worktree，并保留原请求目录相对仓库根的子路径。
- Claude 和 OpenCode 当前只使用项目显式传入的 workspace 或普通 task 目录，不创建 Code Pet 管理的 Git worktree。
- 已有会话继续保留上游返回的原生 cwd，不迁移到新分层，也不从 cwd 反推项目归属。

## 来源

2026-09-06 新建会话选择项目导致 workspaceRoot 缺失的修复，以及用户确认的 CodePet workspace 分层方案。Codex 0.151.0 隔离实测表明 projectId 不会自动解析为 cwd；thread/start 接收已有工作目录。2026-09-08 用户要求固化 Windows/macOS 的安装目录、数据目录和工作区目录边界。

## 验证方式

Codex Provider 的 workspace_mode_tests 检查 task 目录创建、main 路径保持、worktree 路径分层、detached HEAD、唯一目录和子路径保留。Remote 的 remote_home_screen_test 覆盖项目主工作区、worktree 参数和非项目 task 路径。运行 Provider crate 测试及 Remote 会话/Gateway 回归测试；实际安装包生效需重新构建 Provider 和 Remote。
