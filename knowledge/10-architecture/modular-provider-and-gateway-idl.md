# Provider 与 Gateway 按功能组织的协议 IDL

## 背景

Provider 负责插件与 Host，Gateway 负责 UI、手机和其他操作端与 Host。两者共享 common 和 Agent 业务对象，分别编解码；不引入压缩载荷透传。原 Gateway 将 31 个方法和 11 个事件集中在一个 manifest；配对 DTO 挂在 channel/lan 下，实际 WebRTC 邀请也复用配对管理。Provider SDK 迁入时把共享对象置于发布包目录，造成源文件归属倒置。

## 目标

权威源统一放在 `protocol/v1/`，common、agent-domain 中立共享；Provider、Gateway 按功能维护 Schema、methods/events。Gateway 的 pairing 单独成模块，不从属于 LAN。对外 Provider SDK 保持已确认的 `provider-sdk/v1/` 自包含布局，由同一权威输入组装。

## 非目标

不改变现有业务方法名、字段约束、RPC wire、配对流程或通道实现。不新增 Observation，不迁移旧桌面调用，不实现原始压缩包透传，不要求 IPC/WebSocket/WebRTC 全部 Schema 化。

## 现状理解

`tools/protocol-codegen/generate.mjs` 已有规范化模型与 Rust/TypeScript/Dart target。现有 SDK、Host、桌面和外部手机依赖生成类型的旧公开路径。保留这些路径有助于将本次整理限制在协议组织和生成，而不是业务重构。LAN 兼容类型中仍有 HTTPS 地址、证书摘要及历史必填字段；移动来源不等于已经实现新的跨通道 wire。

## 实现路径

`protocol/v1/{common,agent-domain,provider,gateway}` 为权威输入。两侧模块入口只登记文件，不重复登记具体方法；方法与事件只在所属模块出现。跨文件引用必须闭合，Provider 与 Gateway 业务模块不得互相依赖。基础 JSON-RPC 信封由 common 定义，Gateway 事件游标、trace 和本层会话语义独立维护。

Provider 按生命周期、实例、运行时、会话、项目、turn、审批、订阅、usage 分组。Gateway 按 session、subscription、provider、project、conversation、input-queue、turn、approval、usage、pairing 分组。配对的现有传输绑定保留并明确标记，不能为了目录改名使未授权调用进入已认证业务 dispatcher。通道切换控制保持独立。

加载器统一检查 Schema description、引用、依赖方向、重复定义和方法/事件重名。原 core/agent/provider/gateway/channel 类型文件及汇总 manifest 均为生成兼容视图，仓库默认生成直接读取权威模块，不能把旧文件重新作为事实源。SDK 导出复制所需依赖闭包并重写相对引用，不携带 Gateway、配对或通道控制内容。

## 涉及模块

- `protocol/v1/`：共享对象与两层交互协议的唯一来源。
- `tools/protocol-codegen/`：加载模块、校验、生成兼容视图和语言代码。
- `scripts/export_provider_sdk.mjs` 与 staging：维持公开包布局，禁止泄漏内部协议。
- SDK、Host、桌面：保留公开模块路径，验证生成类型和真实进程交互。
- `protocol/README.md` 与知识规约：区分权威输入、兼容产物和发布产物。

## 风险

- 拆分丢失方法、事件或约束：迁移前后结构归一化对比，加原 fixtures 和 Host 互通测试。
- 同名或错误跨层引用被聚合掩盖：加载器拒绝重复类型、重复方法/事件、悬空引用及越层依赖。
- 发布包混入内部合同或引用越出包：执行导出/staging 的引用闭包与文件边界测试。
- 配对归属变化被误解为 wire 升级：保留兼容类型、现有端点与字段；文档显式注明尚未改变通信流程。

## 测试计划

运行 `npm run protocol:check`、`npm run providers:test`、`npm run sdkgen:test`；新增模块来源修改传播、重名、缺失 description、越层依赖与引用闭包测试。执行 Rust Gateway/Provider SDK、Host manager、process RPC、三种 profile、LAN/RTC 相关测试，并检查桌面消费端编译。

## 验证结果

2026-11-12，本机验证完成：

- `npm run protocol:check`：32 项通过；迁移前后 core、agent、Provider、Gateway、配对及通道控制的 Schema/注册表归一化摘要一致。新增测试覆盖模块重名、重复类型、缺失 description、越层依赖及悬空引用。
- `npm run providers:test`：5 项通过，公开 SDK 仍为自包含 provider-sdk/v1，引用闭合且不含 Gateway、配对、Host 代码或 fixtures。
- `npm exec --yes --package=bun -- bun test tools/cp-sdk-gen/cp-sdk-gen.test.mjs`：4 项通过。直接破坏临时目录里的旧兼容 Schema/manifest 后仍能生成 SDK；修改权威 session 心跳策略会同时改变语言常量和摘要。当前 shell 无全局 Bun，因此通过 npm 缓存的 Bun 执行，未新增项目依赖。
- Rust SDK lib：Provider 41 项、transport 12 项、Gateway 1 项通过，另有 5 个原有可选测试忽略。Core/Agent/配对 SDK 编译通过。
- Host lib 107 项通过；manager_gateway 46、process_rpc 8、独立 Provider profiles 2、三个内置 Provider 集成 4、LAN/RTC listener 4 项通过。付费模型及两个原生/公网探针按原有 opt-in 条件忽略。本轮没有重跑真实付费模型。
- Host 全局文件描述符计数测试在首次并行运行时失败；该测试观察整个测试进程的 FD，串行复跑整个 lib（`-- --test-threads=1`）107 项全部通过，未修改测试或运行机制。
- Gateway Dart 10 项通过；修正原测试中写死的旧方法数及遗漏的已有负例类型。四个 Dart SDK `pub get` 后执行 `dart analyze --no-fatal-warnings`，无错误；保留未修改 Agent 生成文件中的两个既有冗余 null 比较 warning。
- Bun 编译的内部生成器实际导出 Gateway Rust/Dart SDK；仓库外 Rust workspace cargo check 与 Dart analyze 通过，独立 Dart 包无 issue。
- `npm run build` 与桌面 `cargo check --manifest-path src-tauri/Cargo.toml --lib` 通过。Tauri build-script 只检查通用依赖 feature，首次直接检查被现有 macOS allowlist 阻挡；检查时临时按 CLI 的方式补齐通用 macos-private-api feature，结束后完整恢复 Cargo.toml，macOS target 原有 feature 未改。
- `git diff --check` 通过。编译使用既有 CARGO_TARGET_DIR 缓存，没有读取其他工作区源码。本轮未重打包或替换已安装 App。

## 知识沉淀

### 两端适配与原生联合验收（2026-11-12，后续执行）

前述记录是目录迁移阶段的结果；后续已完成 Remote 仓库的三份 SDK 同步、两端正常 App 构建和 Android 模拟器联合验收。新增 `scripts/sync_remote_sdk.mjs` 统一生成 Gateway、配对 DTO 与 channel-control，并以同一 canonical protocolDigest 检查新鲜度。配对兼容包名保留，源归属仍在 Gateway pairing；channel-control 只导出独立 DTO。Dart optional+nullable 校验的重复 null 分支已在生成器修复，Remote 静态分析无 issue。

真实桌面 Host 与 Android 原生通道在三种独立 Provider profile 下完成 LAN/WSS、WebRTC 的消息、delta、历史分页、队列、心跳、生产 UI、切会话及撤销凭据验收。Remote 全量 503 项、生成检查 32 项、导出 5 项、SDK 生成/同步 6 项通过；Host profile 2 项及单独授权的 Pi DeepSeek Flash 真实模型调用 1 项通过。插件能力默认值、分页与 emoji 分块修复来自只读取公开 SDK 的独立作者。

具体重现步骤、隔离环境、设备结果与构建产物见 [桌面与 Android 协议联合测试](../40-runbooks/desktop-android-protocol-joint-test.md)。本次是模拟器验收，公网信令、TURN 与实体手机仍未覆盖。它替代下文“尚未重打包/手机端未验收”的阶段性限制，不扩展为真机结论。

同步协议总览与 IDL 唯一事实来源规约。Provider SDK 前一轮验收记录作为历史保留，并注明新权威目录。

## 未知项

目录迁移阶段尚未验证手机端；后续模拟器联合验收与两端构建已完成，证据见上节及 runbook。实体手机、移动数据、公网信令和 TURN 仍未验证。
