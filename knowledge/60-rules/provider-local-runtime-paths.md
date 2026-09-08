# Provider 本地路径和数据目录

## 规则

文件系统路径使用标准路径 API 构造、取父目录和取文件名，禁止拼接目录分隔符。Rust 使用 `Path` / `PathBuf`，Tauri 前端使用 `@tauri-apps/api/path`，Node 使用 `node:path`。URL、协议 ID 与文件系统路径分别处理；构造文件名不等于拼接目录。

运行时可执行文件属于安装位置，配置、历史、缓存属于数据位置，两者不能互相推导。Provider 可以在自身配置 `env.CODEPET_RUNTIME_MIN_VERSION` 中声明最低 Harness semver；SDK 保留所有已检测安装及版本，对低于下限或无法比较的版本标注原因并拒绝选择。最低版本约束只适用于必要的基础协议，不得因缺少项目等可选能力而阻断整个连接；可选能力由实际探测决定是否广告。未配置下限时保持能力探测判断。满足最低版本仍需通过启动和实际接口探测，不能仅凭版本广告能力。CodePet wire 协议版本协商仍是独立契约。

Code Pet App、内置 Provider adapter 和用户安装的 Harness 是三层独立安装：

| 层级 | Windows | macOS |
| --- | --- | --- |
| Code Pet App | NSIS 默认按当前用户安装，通常位于 `%LOCALAPPDATA%/Code Pet/`；安装器选择的位置才是最终权威 | 通常为 `/Applications/Code Pet.app/`，也允许用户把 App 放到其他位置 |
| 内置 Provider adapter | App resource 下的 `provider-plugins/` | `Code Pet.app/Contents/Resources/provider-plugins/` |
| Harness | 独立安装，由发现结果或用户选择的可执行文件绝对路径决定 | 独立安装，由发现结果或用户选择的可执行文件绝对路径决定 |

安装包内的 `codepet-provider-codex`、`codepet-provider-claude` 和 `codepet-provider-opencode` 是 adapter，不是对应 Harness。App 通过 Tauri resource API 定位内置 adapter；开发环境才允许用 `CODEPET_BUNDLED_PROVIDER_PLUGINS_DIR` 覆盖。Host 还会检查 Code Pet 数据目录下的 `provider-plugins/` 和 `providerPlugins.directories` 中的附加目录，但这些扩展目录不改变 App 安装位置。

Harness 的常见安装候选如下；表中只描述发现范围，不把任一候选当成固定安装目录：

| Harness | Windows 候选 | macOS 候选 |
| --- | --- | --- |
| Codex | PATH；`%LOCALAPPDATA%/OpenAI/Codex/bin/<installation-id>/codex.exe`；Microsoft Store package 的 LocalCache；npm 安装 | PATH/登录 shell；`/Applications/Codex.app/Contents/Resources/codex`；`/Applications/ChatGPT.app/Contents/Resources/codex`；npm 安装 |
| Claude Code | PATH；npm package `@anthropic-ai/claude-code` 声明的原生 executable | PATH/登录 shell；`~/.local/bin/claude`；npm 安装 |
| OpenCode | PATH；npm package `opencode-ai` 声明的原生 executable | PATH/登录 shell；`~/.local/bin/opencode`；`~/.opencode/bin/opencode`；npm 安装 |

Windows npm 的 `.cmd`、`.ps1` 和无扩展名 shim 只用于定位 package，Provider 最终保存并启动 package manifest 指向的原生 `.exe`。`CODE_PET_CODEX_BIN`、`CODE_PET_CLAUDE_BIN` 和 `CODE_PET_OPENCODE_BIN` 可以增加显式候选，但选择结果仍规范化为绝对 executable 路径。

## Runtime 数据目录

Runtime 数据目录属于 Provider 实例，和 Code Pet 应用数据目录分别配置。实例 `settings.dataDirectory` 必须是绝对路径：

| Provider | 未配置 `dataDirectory` | 已配置 `dataDirectory` |
| --- | --- | --- |
| Codex | 继承 `CODEX_HOME`，否则使用 `~/.codex` | 作为该实例的 `CODEX_HOME`；`codexHome` 是兼容别名 |
| Claude | 继承 `CLAUDE_CONFIG_DIR`，否则使用 `~/.claude` | 作为该实例的 `CLAUDE_CONFIG_DIR`；`claudeConfigDir` 是兼容别名 |
| OpenCode | 完整继承 Harness 原有 `XDG_*` 和 OpenCode 配置环境 | 作为隔离根，分别派生 `config/`、`data/`、`cache/`、`state/` 并映射为四个 XDG root |

OpenCode 会在 XDG root 下继续使用自己的 `opencode` 子目录。设置隔离根时必须移除继承的 `OPENCODE_CONFIG`、`OPENCODE_CONFIG_DIR` 和 `OPENCODE_CONFIG_CONTENT`，避免显式实例仍混入外部配置。三个 Provider 都必须让启动、历史读取、账号/模型/版本探测使用同一个实例数据上下文。

## 适用场景

内置 Provider 的 runtime discovery/select、实例配置、进程启动、历史扫描、项目归属读取，以及前端本地文件导入。

## 反例

- Windows PATH 中只有 `claude.cmd` 和无扩展名的 Unix shim，检查 `directory.join("claude").is_file()` 后直接启动会选错文件。
- 历史扫描读取 HOME，而 hook 安装读取 Windows 用户目录，导致活动配置与历史目录不一致。
- 探测设置了 CLAUDE_CONFIG_DIR，发送进程没有设置，导致新会话写回默认目录。
- 将 OpenCode 本机 1.18.29 因不等于 1.18.25 拒绝，即使所需接口实际可用。

## 推荐做法

- 共享跨平台基础能力放在 Provider SDK 的 `local_runtime`；产品安装布局由各 Provider 持有，Host 不重新引入产品路径规则。
- Windows 优先原生 `.exe` / `.com`；npm 从 package.json 的 bin 字段解析原生文件。对用户选择的已知 npm shim 做同样解析，不执行任意 shell 文本。旧式纯 JS npm 包没有原生 bin 时，不宣称已支持。
- Unix GUI 进程的 PATH 不能代替用户交互 shell 的 PATH；npm/nvm 常只在 `.zshrc` 中初始化。后台读取交互登录 shell 的 PATH，再查找真实文件；不要将 `command -v` 的函数名或带启动提示的完整 stdout 当作可执行路径。保持 shell 超时和进程树清理。
- 发现 CLI 所用的 shell PATH 必须同时用于版本扫描、异步信息探测和实际 Harness 子进程；绝对路径只能定位入口脚本，不能让 `#!/usr/bin/env node` 自动找到解释器。SDK 在后台发现时保存 PATH 快照，统一进程入口只向子进程注入快照，不修改 Host/Provider 全局环境，也不在异步启动路径重复运行 shell。重新检测时刷新快照，Windows 继续使用自身环境。
- 登录 shell 的 stdout 必须在进程运行时并发读取，不能先等退出再读取；启动输出可能填满管道，造成假超时。枚举 shell PATH 的全部匹配项，再按规范化路径去重，不能只保留第一个命令。
- 使用平台目录库解析 home；专用环境变量仍可覆盖数据目录。自定义 runtime 选择应保留在安装列表，即便不在 PATH。
- 实例 `settings.dataDirectory` 为绝对路径；Codex 映射 CODEX_HOME，Claude 映射 CLAUDE_CONFIG_DIR，OpenCode 映射独立 XDG config/data/cache/state 子目录。未配置时沿用 Harness 原有环境和默认位置。
- 对实例设置的目录，启动、账号/模型探测和读取历史使用同一上下文；不要修改 Host 进程全局环境来切换实例目录。
- 不要把 Code Pet 的 `settings.data.dataDirectory` 自动当成任一 Harness 的 `dataDirectory`。两者生命周期和内容不同，必须由调用方分别传递。
- 不要把 Harness 数据目录当成会话 workspace；Code Pet 管理的会话目录由 [Provider 新建会话的项目与工作目录](provider-conversation-workspaces.md) 规定。

- 安装扫描和版本子进程不得阻塞异步 RPC 执行器，也不得持状态锁等待探测。PATH 目录及规范化后的可执行文件先去重，再探测；Provider 初始化后后台扫描，runtime.getInstalled 只读快照，完成后用 runtime.inventoryChanged 通知 Host；不得把整轮扫描塞进 RPC 超时窗口。
- 后台 Provider、Harness、登录 shell 和探测进程统一通过 SDK process 业务接口管理启动、等待、终止和清理。平台实现分别位于 process/windows.rs 与 process/unix.rs：Windows 用 CREATE_NO_WINDOW 和 Job Object，Unix 用进程组；Provider 不得直接拼 taskkill 或调用 libc::kill。

## 来源

- [v0 Windows 审阅](../10-architecture/windows-platform-review.md)
- [Windows Provider 验证路径](../40-runbooks/windows-provider-runtime.md)
- [设置持久化](../10-architecture/settings-persistence.md)
- 2026-09-06 曾取消 Harness 版本范围；2026-09-08 用户明确要求增加 Provider 最低版本约束，本次指示取代旧限制。
- 2026-09-08 用户要求固化 Windows/macOS 的安装目录、数据目录和工作区目录边界。

## 验证方式

SDK `local_runtime` 测试覆盖 `.exe`、npm shim、空格/中文路径、OpenCode XDG 映射和路径越界拒绝；三个 Provider 的 storage_path_tests 覆盖安装/数据分离与相对路径拒绝。Plugin Catalog 测试覆盖 bundled、应用数据目录和附加目录。`native_runtime` 忽略测试用于真实二进制、隔离数据目录验证；不要将本机 smoke 描述为所有版本和所有操作已验证。macOS App 可移动，因此发布验收应通过 Tauri resource API 结果检查 adapter，不能只检查 `/Applications` 的字面路径。
