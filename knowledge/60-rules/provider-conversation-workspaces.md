# Provider 新建会话的项目与工作目录

## 规则

项目归属和执行目录分别传递。CodePet 管理的目录位于 Host 用户的 `~/.codepet/remote_workspace/<provider>/`，非项目任务使用 `task/<任务ID>/`，工作树使用 `worktree/<工作树ID>/`。

## 适用场景

Remote 新建会话、Provider 的 conversation.create 与 Codex thread/start 参数转换。defaultWorkspaceRoot 保持 Provider 层，不能改成 task 层后又由 Remote 重复拼接。

## 反例

只传 projectId 会丢失执行目录，App Server 不保证按项目根目录启动。把 CodePet 工作树放进 CODEX_HOME/worktrees 会混入 Codex 桌面端管理范围。用工作树目录反推项目归属会使会话脱离原项目。

## 推荐做法

Remote 从所选项目取根目录，连同 routed project 和模式提交。main 直接使用该目录；worktree 由 Codex Provider 创建独立 detached 工作树，再以新目录调用 thread/start，同时保留 projectId。项目子目录要保留相对路径。非项目目录根据 Provider 广告的根路径分配，其他 Provider 仅广告实际支持的模式。新规则只作用于新建目录，不迁移已有会话。

## 来源

2026-09-06 新建会话选择项目导致 workspaceRoot 缺失的修复，以及用户确认的 CodePet workspace 分层方案。Codex 0.151.0 隔离实测表明 projectId 不会自动解析为 cwd；thread/start 接收已有工作目录。

## 验证方式

Codex Provider 的 workspace_mode_tests 检查 task 目录创建、main 路径保持、worktree 路径分层、detached HEAD、唯一目录和子路径保留。Remote 的 remote_home_screen_test 覆盖项目主工作区、worktree 参数和非项目 task 路径。运行 Provider crate 测试及 Remote 会话/Gateway 回归测试；实际安装包生效需重新构建 Provider 和 Remote。
