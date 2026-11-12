# Pi 外部 Provider：SDK 实证与验收记录

> 后续验收：本文保留早期 TS/Rust bridge 实验路径。新版三 profile、纯合同 SDK 和无 Rust bridge 的独立 pi 验收见 [传输与合同 SDK 实现](../10-architecture/provider-transport-profiles-and-contract-sdk.md)。新作者应使用 `protocol/provider/docs/README.md`，不沿用旧 App 生成器入口。

## 背景与范围

2026-11-11，用户要求先用 Python 或 TS 实现一个 pi Provider，经自定义添加装入 App，再反推 SDK 是否合理。实现位于 [独立示例](../../examples/providers/pi/README.md)，没有加入内置 Provider 清单。本轮先执行外部接入验证，作为 [局部整理计划](../10-architecture/observation-and-sdk-refactor-plan.md) 的前置基线。任务谱系不在范围内。

目标是区分三件事：现有协议能否表达真实 Agent；第三方能否用分发 SDK 开发；原生产品入口能否完成安装。不能用内置 fake-provider 或仅导入成功替代真实调用。

## 证据

- 本机没有 `pi` CLI 和 pi 配置目录。使用官方 npm 包 `@earendil-works/pi-coding-agent@1.5.2`，通过其 `createAgentSession` / `ModelRuntime` / `SessionManager` 调用真实 pi 引擎，未另写假 Agent。
- 已安装 App 的 `/Applications/Code Pet.app/Contents/Resources/provider-sdk/cp-sdk-gen --help` 只支持 Rust/Dart；Provider server 限 Rust。使用该**已安装生成器与 protocol**导出 core/agent/provider 三 crate，在示例 `.build` 独立 Cargo workspace 编译 bridge。它不依赖仓库内置 Provider 或 Host。
- TS 只有 canonical 生成 DTO、没有服务 runtime 和分发目标。示例从仓库复制三份类型，并补一个包内 JSONL worker。bridge 的方法转发从已分发 Provider manifest 生成，未再手写 33 项方法表。
- 首次完整联调验证了返回 DTO 的强校验：ProviderResourceId 与共享领域 RoutedResourceId 不同，PageInfo 字段是 nextCursor；TS 编译和真实 Host 测试分别约束这两种协议形态，不能只凭手写对象猜字段。
- `external_pi_provider.rs` 调用生产 `PluginManager::import_plugin`，经 `dispatch_for_local_client` 走同一 Gateway。通过真实进程的 mux/Initialize、实际 pi read 工具读取 proof.txt 后把结果交给模型端、流式输出、完整工具/消息 item、同 ID 去重、中断、restart 后历史、私有 SQLite、stderr 落盘、同 ID 重复导入拒绝。
- 确定性模型端为隔离的本机 OpenAI-compatible SSE fixture：最终一轮为 3 次模型请求、1 次实际工具结果。在此之外，用户指定 `.zshrc` 的 `DS_API_KEY` 与 Flash 后，新增显式 opt-in 云端验收，按 [官方模型 ID](https://api-docs.deepseek.com/en/) 调用 `https://api.deepseek.com` 的 `deepseek-flash`。真实 Gateway → bridge → TS → pi → DeepSeek → read 工具循环完成，精确返回 `PI_VALIDATION_OK`，item ID 一致，Provider restart 后历史仍在；该轮测试 5.82 秒通过。
- 通过 CUA 在已安装 Code Pet 的“连接接入 → 本机运行时 → 添加自定义 Provider”选择产出 manifest，界面显示“Pi (TypeScript) 已导入并加载”，自定义卡片显示 Provider 在线、Harness 可用、Provider 0.1.0、pi 1.5.2 及安装后的模块路径。插件由原生入口复制至 `~/Library/Application Support/code-pet/versions/providers/dev.codepet.example.pi`，provider.ts、server.ts、bridge 三文件 SHA-256 均与构建包相同。没有替换 App 二进制。随后给本机 pi 新建无明文密钥的 models.json，在请求时通过 zsh 读取用户已有 DS_API_KEY，正常重启 Code Pet，原生卡片再次确认 Pi 在线、Harness 可用；已安装包能识别 `codepet-deepseek/deepseek-flash`。

## 设计判断

现有 Host、导入机制、Provider 业务协议和 Rust runtime 足以承载新的 Agent；本例没有要求 Gateway/通道进入 Provider。Bootstrap 由 SDK 完成，业务 Initialize 将目录传给 TS；stderr 在业务初始化前即可继承 Host 的日志收集。私有 SQLite 可直接使用，不必为了支持 TS 先合并数据库。

缺口集中在扩展开发工具链，而不是需要重写 Host：

| 实证缺口 | 局部收敛方向 | 验收标准 |
| --- | --- | --- |
| 非 Rust 作者缺少对应语言的 mux runtime | 补齐 TS 原生 Provider runtime，由其实现既有 wire 合同；Rust bridge 仅作临时实证，不升级成 TS SDK 的必需组成 | TS Provider 无需 Rust 构建链或 Rust 子进程，独立产物直接启动；跨语言协议一致、异常退出无残留进程 |
| TS 类型和运行时不能从 App SDK 完整导出 | provider profile 导出所需 common、生成 DTO、可选 runtime 与最小模板 | 在无仓库源码目录构建、编译和验收示例 |
| Initialize 身份、目录、资源成功提交仍需手写 | 提供共享验证/资源装配入口，失败不提交初始化状态；业务 schema 留给 Provider | 失败初始化、重试、换身份/目录和 shutdown 测试 |
| runtime inventory 对 bundled SDK 的说明不足 | 文档区分“捆绑模块”与“扫描外部 CLI”；按实际能力展示 | 自定义 Provider UI 显示模块路径/版本，无虚假的扫描或选择能力 |
| 开发者容易混淆两种 resource ID / 能力与分页字段 | 在公共入口提供明确类型与小型转换例子，不把所有领域类型抹平 | 类型检查加真实 Gateway 合同测试 |
| 包含生产依赖约 161 MB，引用本机 Node 绝对路径 | 明确平台/运行时依赖与打包策略，再决定是否分发语言运行时 | 干净机器可安装并启动；Windows/Linux 分别验收 |

bridge 留在示例，尚未升格为公共 SDK API。尤其不能把包内 JSONL 再声明成 Host 的另一种 transport。Python 仍需独立验证对应语言的 SDK 与打包路径，不能据此宣称已支持 Python SDK。Observation-only 的 descriptor 与发现问题仍需独立验证，本例的 Agent 能力成功不能证明该能力边界已经完成。

### 对实现语言与生成边界的修正

用户指出 TS 可打包成独立可执行产物，这与当前进程边界一致。Host 的 PluginDescriptor 已有 executable、args、env，进程启动不识别 Provider 的源码语言；直接二进制场景可沿用这些字段，不因 TS 新增一种宿主。解释执行场景的 runtime 声明是否扩展 manifest，应依据实际发布需求另行评估。

JSON Schema 已由仓库生成器产出 TS interface；本示例复制的是这些生成物，并未手写领域接口。真正缺口是 App SDK 的 TS 导出和 TS 服务 runtime。数据类型/校验可从 schema 生成，方法/事件及 dispatch 映射还需协议注册表；帧、握手、复用、背压和生命周期由语言 runtime 实现。这三部分不能混为一项。

本次桥接和云端调用证明业务合同及 Host 接入可用；尚未证明独立 TS SDK 已成立。下一次验收必须以 TS 自己接入现有 wire、打包产物由 manifest 直接启动、脱离 Rust 工具链和仓库源码构建为标准。Pi 动态依赖与资源是否都能放入单个可执行文件仍需实际打包验证。

## 涉及模块与原因

- `examples/providers/pi/provider.ts`：真实第三方业务实现，覆盖生命周期、能力、会话、工具/消息投影和 Provider 私有存储。
- `examples/providers/pi/bridge/`、`server.ts`：验证当前 Rust-only SDK 如何承载 TS；是实验性公共机制候选，不含 pi 业务特例。
- `build.mjs`：从已安装 SDK 导出，构建并产出可被现有导入器复制的普通文件包；未加内置 staging。
- `crates/codepet-host/tests/external_pi_provider.rs`：验证生产导入与 Gateway，不修改 Host 实现。
- 架构推进计划：将外部包验收前置，后续抽象由本例反推。

## 复现与验证结果

完整命令见示例 README。2026-11-11 已执行：

- `npm run check --prefix examples/providers/pi`：通过。
- `npm test --prefix examples/providers/pi`：3 项通过，覆盖真实 pi 引擎、存储/中断/路由与 JSONL 边界。
- `node examples/providers/pi/build.mjs /tmp/codepet-pi-validation/pi-provider`：使用已安装 SDK 独立构建成功；产出 manifest，递归普通文件检查通过。
- `node examples/providers/pi/tests/host-validation.ts /tmp/codepet-pi-validation/pi-provider`：外部包生产 Host/Gateway 验收 1 项通过，明确使用本机模型 fixture。
- `node examples/providers/pi/tests/live-validation.ts /tmp/codepet-pi-validation/pi-provider`：真实 DeepSeek Flash 的工具、流式消息与持久化验收 1 项通过；显式 opt-in，不放入默认测试。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway import::`：原有 4 项导入回归通过。
- `rustfmt`、`git diff --check`：通过。Host 有既存 `RecentSnapshots::page` dead_code 警告。

本机为了复用 Cargo 缓存给 Host 命令指定既有 `CARGO_TARGET_DIR`，不影响运行时临时隔离目录。构建日志 `/tmp/codepet-pi-build.log`、联调日志 `/tmp/codepet-pi-host.log`、旧导入回归 `/tmp/codepet-pi-import-regression.log`、云端验收 `/tmp/codepet-pi-live.log`；临时 fixture、测试安装与认证配置结束后删除，产出的导入包保留。

## 风险与后续验收

- 桥接会新增 Node 子进程及额外编码/内存开销：目前有 16 MiB record 上限、64 项队列、55 秒调用超时、typed 结果验证、SDK process-tree owner；在升格 SDK 前补 worker 崩溃、挂死、背压、大消息及进程清理压力测试。当前小负载成功不等于生产容量达标。
- Pi 原生 session 与 Provider 索引需要保持一致：已验证正常 restart、保留中断历史；尚未做 kill -9/磁盘故障恢复。正式发布前增加崩溃恢复用例。
- 示例只声明只读工具、禁用自动资源扩展；不是操作系统沙箱。写入、审批、Observation、用量/未读等均未实现，不扩大宣称。
- 原生导入、卡片状态、版本与路径已验收；真实认证和会话已在 Host/Gateway 测试中通过，手机/TUI 客户端页面尚未验证。UI 显示 Harness 可用表示模块已就绪，不单凭这个标记证明模型调用通过。
- 本机 pi 模型配置通过 credential command 使用 DS_API_KEY；zsh 启动脚本的普通 stdout 必须与密钥输出分离，验收脚本用独立 fd 捕获，pi 配置命令也隔离了启动输出。没有复制密钥到 manifest、仓库或报告。本机 `~/.pi/agent/models.json` 为新建文件，没有覆盖已有配置。

## 知识沉淀与未知项

本记录是外部开发者路径的基线，后续 SDK 整理必须保持该示例可构建和联调。当前不新增全仓规约；机制升格后再把经过验证的语言 runtime 与分发约束沉淀到 SDK 规约。尚未验证 Windows/Linux、手机 UI 和 TUI；真实云端已验证用户指定的 DeepSeek Flash，不将它们记为通过。
