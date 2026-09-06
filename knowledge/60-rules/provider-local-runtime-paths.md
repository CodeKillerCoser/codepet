# Provider 本地路径和数据目录

## 规则

文件系统路径使用标准路径 API 构造、取父目录和取文件名，禁止拼接目录分隔符。Rust 使用 `Path` / `PathBuf`，Tauri 前端使用 `@tauri-apps/api/path`，Node 使用 `node:path`。URL、协议 ID 与文件系统路径分别处理；构造文件名不等于拼接目录。

运行时可执行文件属于安装位置，配置、历史、缓存属于数据位置，两者不能互相推导。Provider 不用 Harness 版本白名单、精确版本或版本范围决定可用性；版本用于展示，启动和实际接口探测用于判断能力。CodePet wire 协议版本协商仍是独立契约。

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
- 使用平台目录库解析 home；专用环境变量仍可覆盖数据目录。自定义 runtime 选择应保留在安装列表，即便不在 PATH。
- 实例 `settings.dataDirectory` 为绝对路径；Codex 映射 CODEX_HOME，Claude 映射 CLAUDE_CONFIG_DIR，OpenCode 映射独立 XDG config/data/cache/state 子目录。未配置时沿用 Harness 原有环境和默认位置。
- 对实例设置的目录，启动、账号/模型探测和读取历史使用同一上下文；不要修改 Host 进程全局环境来切换实例目录。

- 安装扫描和版本子进程不得阻塞异步 RPC 执行器，也不得持状态锁等待探测。PATH 目录及规范化后的可执行文件先去重，再探测；Provider 初始化后后台扫描，runtime.getInstalled 只读快照，完成后用 runtime.inventoryChanged 通知 Host；不得把整轮扫描塞进 RPC 超时窗口。
- 后台 Provider、Harness、登录 shell 和探测进程统一通过 SDK process 业务接口管理启动、等待、终止和清理。平台实现分别位于 process/windows.rs 与 process/unix.rs：Windows 用 CREATE_NO_WINDOW 和 Job Object，Unix 用进程组；Provider 不得直接拼 taskkill 或调用 libc::kill。

## 来源

- [v0 Windows 审阅](../10-architecture/windows-platform-review.md)
- [Windows Provider 验证路径](../40-runbooks/windows-provider-runtime.md)
- 2026-09-06 用户明确要求取消 Provider 的 Harness 版本支持范围。

## 验证方式

SDK `local_runtime` 测试覆盖 `.exe`、npm shim、空格/中文路径和路径越界拒绝；三个 Provider 的 storage_path_tests 覆盖安装/数据分离与相对路径拒绝。`native_runtime` 忽略测试用于真实二进制、隔离数据目录验证；不要将本机 smoke 描述为所有版本和所有操作已验证。
