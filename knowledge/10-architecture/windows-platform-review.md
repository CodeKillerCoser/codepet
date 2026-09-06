# v0 Windows 路径与平台能力审阅

> 后续路径修复已完成实现：共享原生可执行文件发现、Windows home 解析、安装/数据目录分离，以及取消 Harness 版本限制。以下保留 ca78541 审阅时的证据；验证与配置见 [Windows Provider runbook](../40-runbooks/windows-provider-runtime.md)。抠图、IPC、应用激活和中断能力缺口仍未补齐。

## 现象与范围

2026-09-06 审阅 `v0` / `ca78541`。本记录基于源码、附近测试和 Windows 最小实验，未运行完整桌面应用。实现代码未修改；问题引入历史未确认。

重点覆盖 provider 运行时发现、配置目录、激活、图片抠图和 Desktop IPC。目录解析问题与主动声明不支持的平台能力应分别处理。

## 证据

### P1：三个 provider 的运行时发现遗漏 Windows 可执行文件

- `crates/providers/codepet-provider-codex/src/provider.rs:3013` 的 PATH 候选使用 `directory.join("codex")`。
- Claude `provider.rs:2081` 和 OpenCode `provider.rs:2200` 同样直接拼接无扩展名命令。
- 后续 `inspect_runtime_candidate` 先执行 `is_file()`；Windows 不会在这里自动补 `.exe`。登录 shell 回退使用 `/bin/zsh -lc command -v`，也没有 Windows 实现。
- `runtime_get_installed` 和未指定 candidate 的 runtime 配置调用这些发现函数；因此仅在 PATH 安装 `.exe` 的用户会得到空安装列表或自动选择失败。旧 `src-tauri/src/agent/runtime.rs` 已有 Windows 发现逻辑，但新 provider 没有复用。

### P1：Claude 历史目录只识别 HOME

- `crates/providers/codepet-provider-claude/src/provider.rs:2240` 在没有 `CLAUDE_CONFIG_DIR` 时只读取 `HOME`。
- `refresh_discovered_conversations:334` 无目录就直接成功返回，跳过历史扫描。
- 同一 provider 的 hook 配置通过 `codepet_observation::home()` / `dirs::home_dir()` 解析；Windows 上 hook 可以装到用户目录，而历史列表为空。这是同一数据目录使用两套平台规则的实际不一致。

### P2：Codex 项目归属目录只识别 HOME

- `crates/providers/codepet-provider-codex/src/provider.rs:2713` 在没有 `CODEX_HOME` 时仅回退 `HOME`。
- `load_codex_desktop_project_assignments:3770` 无目录就返回空归属映射；`conversation_list:1915` 使用该映射进行项目成员筛选。
- 仅设置 `USERPROFILE` 的 Windows 环境会忽略已有 `.codex-global-state.json`，影响依赖桌面归属映射的筛选，不等于所有会话都无法列出。

### P2：抠图入口没有平台能力门控

- `frontend/App.svelte:2075` 始终提供抠图导入按钮，只根据 busy 状态禁用。
- `src-tauri/src/pet/subject_cutout.rs:54` 所有非 macOS 平台直接返回 only supported on macOS。
- Windows 用户选完图片后必然失败；应提供 Windows 实现，或根据后端能力隐藏/禁用入口并说明原因。

## 已确认的平台能力缺口

- **Desktop companion**：`src-tauri/src/agent/codex_desktop_ipc/transport.rs` 仅实现 Unix socket；非 Unix 返回 Unsupported。Windows 无法使用这条本地跟随/控制通道。它与独立 App Server 是两条通道，不能把 App Server 可用当作 companion 已支持。Windows 侧实际传输协议未验证。
- **Claude 中断**：`client.rs:162` 非 Unix 返回 InterruptUnsupported；`provider.rs:1742` 只在 Unix 声明 TurnInterrupt。属于明确且正确门控的功能缺口。
- **按应用名激活**：`src-tauri/src/agent/actions.rs:371` 非 macOS 直接报不支持，Qoder/Cursor 默认仍生成 AppName。前端旧活动能力模型仍返回 canActivate=true；当前可达 UI 场景需进一步实机核对。
- **悬浮窗口特殊行为**：`src-tauri/src/lib.rs:555` 非 macOS overlay 配置为空。不能仅凭这一点判定窗口故障，因为通用 Tauri 配置仍会生效；需要 Windows 多显示器、焦点与穿透验证。

## 验证结果与已排除方向

执行 `git status --short`、`git rev-parse --short HEAD`、按目录的 `rg` 检索和源码/测试阅读。

在系统临时目录提取当前源码的 `codex_home`、`claude_config_dir`，使用 `rustc --edition 2021 --test` 编译并运行测试程序（`--test-threads=1`）：2 项通过。第一项仅在子进程清除 HOME 和两个专用配置变量、设置 USERPROFILE，确认两个函数都返回 None；第二项创建 codex.exe，确认无扩展名候选的 is_file 为 false。测试通过表示成功复现缺陷，不表示实现正确。

未运行仓库完整 Cargo/Tauri 测试和桌面 UI。没有把测试 fixture 的 `/tmp`、macOS 条件编译路径或普通 Path::join 中的 `/` 当作独立缺陷；也未证明任意混用分隔符都会在 Windows 失败。

## 修复方向与验证路径

1. provider/SDK 内复用跨平台运行时发现，保留既有模块边界；用仅有 `.exe` 的 PATH、空 PATH、显式配置和 Windows 安装布局验证自动发现及选择。脚本包装器需单独验证启动方式。
2. 统一配置目录解析及变量优先级；覆盖仅 USERPROFILE、显式 override、空变量，并验证 Claude 历史和 Codex 项目归属真实 fixture。
3. 抠图和应用激活使用后端能力契约；分别验证 Windows/macOS 的动作可见性、禁用原因、失败提示和焦点恢复。
4. IPC 和中断能力补齐前继续明确声明不支持；新增实现后验证真实连接、取消语义和子进程清理。

## 未知项

未验证 Windows Codex Desktop 的实际 IPC 协议、安装渠道差异及完整 GUI 行为。暂不对这些事项提出确定的适配代码。修复落地后应更新本记录及跨平台边界文档，并把上述目录解析和运行时发现案例纳入共享回归测试。
