# Observation 与 SDK：局部整理与收敛推进计划

状态：2026-11-11，局部整理方案已获认可；按用户补充先执行 [Pi 外部 Provider 实证](../40-runbooks/pi-external-provider-sdk-validation.md)，后续已执行传输/SDK 范围，当前状态见下一段；其余业务拆分仍为计划。任务谱系排除在本轮外，也不在本轮删除。本文替代早期的大范围迁移设想，证据见 [模块边界 Review](current-module-boundaries-review.md)。

后续明确的分发合同覆盖本文早期“随 App 携带运行时/生成器”的设想：App 仅分发 JSON Schema、methods/events、profile 和作者说明。当前传输与 SDK 范围已落地：独立 crate、三种 profile、跨 Provider 并发和纯合同分发，详见 [实现与验收](provider-transport-profiles-and-contract-sdk.md)。本轮不据此宣布完整 Observation 拆分完成。

## 背景

当前 Host、Gateway、独立 Provider、stdio bootstrap 和共享通道会话已有清晰基础。问题集中于重复初始化规则、部分模块归属、桌面客户端接口/事件尚未统一，以及 App 中扩展 SDK 的分发范围。整理应沿已有接缝推进，阶段独立验证和回滚。

## 目标与边界

| 边界 | 本轮目标 |
| --- | --- |
| common | 统一共享类型的使用入口；保留基础与领域模块层次，不强制先物理合并所有 crate |
| gateway | 官方桌面、手机、未来 TUI 共用业务协议；请求、事件与错误一致，具体 transport 可注入 |
| provider | 扩展能力实现入口，公共生命周期复用，Agent 控制与 Observation 可独立声明和启停 |
| App 扩展 SDK | 仅 provider 与必需 common；Gateway、LAN/RTC 物料不进入该分发 profile |
| 通道 | 应用内部装配 IPC/WSS/RTC，传送请求、响应、事件并处理连接生命周期；通用部分不依赖特定通道 |
| 平台能力 | 窗口、剪贴板、权限等保留 Tauri/platform adapter，业务页面不散落平台命令 |
| 存储 | 先统一路径配置和数据所有权；保留现有 Host 共享库与 Provider 私有库，不合库或重写 SQL |

## 非目标

不接入桌面完整对话、不建设第三方 RTC 开放平台、不新增独立 Host 进程或本地 IPC server、不拆新插件宿主、不做插件市场。任务谱系不迁移、不新增接口、不删除，既有功能只作为全仓编译的兼容消费者处理。三个原生 Agent 的执行模型、现有传输 bootstrap 和网络 wire 不因代码整理而重写。

## 现状理解

外部验收前置：先用 TS pi Provider 经自定义包导入验证开发、加载与实际调用。当前已通过真实 Host 导入/Gateway/SDK 子进程联调及原生 App 自定义入口导入；真实 DeepSeek Flash 工具调用与消息持久化也已验收。后续阶段 2/3 的公共抽象应消除这个示例中重复的初始化、语言 bridge 和类型/打包工作，不能只以三个内置 Rust Provider 为唯一依据。

TS 验收目标：Provider 使用 TS 原生 SDK runtime 接入既有进程协议，构建为 Host 可按 executable/args/env 直接启动的产物；不要求 Rust bridge。JSON Schema 自动生成 DTO/校验，协议方法与事件注册表自动生成映射，SDK 承担传输及生命周期实现。已有桥接示例是业务链路证据，不能作为“独立 TS SDK 已可用”的完成标记。

- bootstrap 已在 Provider SDK transport 中完成版本/容量协商；provider.initialize 负责 Host 身份、目录及业务装配，三 Provider 存在可复用校验和状态边界问题。
- LAN/WSS 与 RTC 已复用 channel session 和 Gateway；通用诊断却放在 webrtc 子目录，LAN listener 还承担部分远程运行时装配。
- canonical Tauri 请求已到同一个 Gateway，但前端直接 invoke、事件桥与网络 session 合同不同；Pet 等仍使用独立协议。
- Host 共享 app-state schema 位于 provider-data，私有 usage.sqlite 同时存在；本轮先整理归属和配置，不改变数据位置。
- App SDK staging 复制所有相关协议，带入 Gateway/channel/fixture/RTC 文档；生成库本身并未因此依赖 LAN。

上述为静态源码证据；初始化异常是否形成用户可见故障尚未实机复现。上一轮现有协议、staging、OpenCode 观察插件检查 30 项通过，不能替代后续改动验证。

## 实现路径与阶段验收

### 阶段 0：固定范围、基线与实际调用入口

**工作**：列出实际消费的 Tauri 业务/管理/平台/兼容接口；任务谱系标记排除。修正运行拓扑和前后端边界文档中的旧 collector/Companion 现行描述。记录当前 wire 样例、SDK 导出布局、两端版本、旧接口消费者及已知失败。

**涉及模块与原因**：knowledge 的运行拓扑/边界文档、src-tauri/src/lib.rs 命令注册、frontend API 封装；避免选错现行入口。只建立任务所需清单，不创建全仓中央知识映射。

**通过条件**：清楚区分已有机制与拟新增能力；每个旧入口有消费者证据或“待确认”标记。已有用例作为后续回归基线，环境无法执行的检查明确记录，不反复拿历史通过结果当新验证。

### 阶段 1：通道公共设施归位

**工作**：把通用日志 sink、连接标识和非阻塞输出移到 channels 下的中性模块；ICE/SDP/PeerConnection 统计留在 webrtc。通过小的装配入口明确 listener、session registry、signaling controller 的所有权。保留现有网络协议、默认行为和容量限制；先不增加复杂的自动启停策略。

**涉及模块与原因**：Host remote/channels 的 diagnostics、session、lan/listener 和 webrtc/cloud/service；解除 LAN/session 对 RTC 诊断目录的反向依赖，表达真实装配边界。不是另写一套会话服务。

**验证/通过条件**：日志不阻塞、敏感内容不泄漏，兼容已有日志字段或显式迁移；LAN 与 RTC 的请求/事件、热切换、撤销和关闭行为不变。通用 session 不再引用 webrtc 私有诊断。

### 阶段 2：统一 Provider 初始化公共规则

**工作**：使用现有 bootstrap/serve_stdio，抽取小型初始化上下文与版本/身份校验；三 Provider 共享“未初始化/进行中/成功/失败”语义。先准备业务资源，再提交对外成功状态；失败时清理已启动资源，规定同配置重试和变更配置的处理。

日志 bootstrap 与业务配置分开：进程启动前 Host 已能捕获 stderr；data/logs/private database 等业务配置通过 initialize 有唯一权威来源。共享元数据库环境变量先归并为明确的内部配置，不在这一阶段强制改为跨进程存储 RPC。简单观察扩展可不使用数据库。

**涉及模块与原因**：SDK runtime/公共 init helper、三个 provider.rs、Host manager 配置和 provider-data 初始化；消除复制和两种 ready 判定。scanner、原生协议和 SQL 仍归各自实现。

**验证/通过条件**：三 Provider 运行同一组版本/身份/重复初始化合同测试；无效目录失败后重试、并发 initialize、失败后业务调用、路径重绑定有明确结果；bootstrap、心跳、取消、EOF/shutdown 正常。只初始化观察不会启动 Agent harness。

### 阶段 3：IDL 与 Provider-only SDK 物料整理

分成三个可单独验证的提交组，不与后续业务扩展捆绑：

1. 协议 manifest 职责拆分：schema 定义 DTO，methods 定义方法/能力/幂等性，events 定义事件；版本/依赖/wire 公共规则由生成配置维护。fixture 索引转入测试入口。插件安装 manifest 与产物 sdk.json 保留各自名称和职责。
2. common 先统一公共导出和模型权威位置；既有 core/agent 可暂作内部组件，Gateway 与 Provider 继续共享同一具体类型，不制造重复 DTO。是否物理合包以后以实际依赖和导出复杂度判断，不作为本轮必需前置。
3. cp-sdk-gen 和 staging 增加 App Provider-only profile：只暴露扩展所需目标、说明、输入和运行时；Gateway 生成能力留仓库内部。fixture 默认不分发，源码 CI 仍执行全部正负例。离线生成器内的目标/模板也按 profile 检查，不只删除外部目录。

**涉及模块与原因**：protocol/codegen.json、tools/protocol-codegen、tools/cp-sdk-gen、运行时文件清单、scripts/stage_provider_plugins 及测试、package.json、Tauri resource 配置和 CI。处理的是生成输入与分发边界，不强制改业务 wire 或发布 Provider v2。

**构建目标**：源码 IDL/fixture 校验 → 单 IR → 干净目录生成 → 加必要手写运行时与包描述 → 独立验证 → 白名单 staging → App。仓库 sdk/ 和分发输出共用同一生成器；发布不从整个 protocol/ 递归复制。App 保留离线导出能力，目录可继续叫 provider-sdk；命名调整不作为目的本身。

**验证/通过条件**：生成新鲜度、错误引用/重复方法/负例拒绝、语言与 role 的 fail-closed 检查通过；App 物料不含 Gateway/channel/RTC 文档或默认 fixtures；从最终 bundle 离线导出并在仓库外编译 Provider，依赖完整、无工作区绝对路径。内部 Gateway 及手机生成包继续编译。checksum、目标版本及输入依赖闭包 digest 可核对；不要求不同目标的 digest 相同。

### 阶段 4：收敛 Gateway 客户端请求与事件合同

**工作**：在已有 canonical Gateway 前端封装外建立应用内部可注入 transport，集中 request/events/readiness/close，把 Tauri invoke/listen 限定在 adapter。先迁移已有会话、用量、队列调用，不增加 TUI 产品或新通信服务。

事件合同必须明确：先订阅与快照 fence 的顺序、cursor/replay、订阅关闭、页面重挂/前端刷新、流缺口恢复、迟到响应隔离，以及连接状态和错误。复用手机已实现行为与测试场景作为参照，不重新设计分页/缓存。不同语言不宣称能直接共用同一份客户端源代码。

服务端先创建 Host/Gateway，再按配置挂 adapter；客户端先建立通道/上下文，再发业务请求。Tauri 使用可信本地上下文，网络保留 credential/握手。保留本地请求级在线 guard，不能为了统一改成永久在线；取消 RPC 不承诺撤回已受理的 Agent 任务。

**涉及模块与原因**：frontend/lib/codepetGateway.ts、Gateway 生成客户端/transport contract、tauri_bridge canonical 请求和事件桥、Host 调度与订阅边界；当前共享业务服务，但客户端合同尚未透明替换。

**验证/通过条件**：同一业务在本地与网络 adapter 合同测试中得到一致数据/错误；覆盖并发请求关联、取消、初始化中、无手机连接、本地 guard 释放、快照/事件衔接、cursor 失效、页面重挂和订阅清理。网络不能使用可信本地准入。用户已有手机行为保持通过。

### 阶段 5：Observation 与桌宠入口逐组收敛

这是最小能力扩展，需要独立审查，不伪装成纯文件搬迁；阶段 1–4 的整理成果可先交付。

**5A 来源能力**：复用 codepet-observation 的安装/互斥/接收；增加明确的 Observation 能力声明，Host 按能力发现来源。纯观察插件无需声明虚构 Agent instanceKinds，亦不参与 Agent inventory/start/releaseControl。先以本地 fixture 插件证明该边界，再迁移三个内置来源。

**5B 信息与投影**：从当前 Pet projection 迁移来源原生解析和任务归并，输出最小标准 activity/message/request 信息；保留原有轮次、子代理、终态和缺口规则。动态来源替换固定列表，Pet 只处理展示。订阅 generation/stream/fence 和有界状态必须可验证；Hook 无法恢复的缺口保持 unknown。

**5C 客户端入口**：桌宠快照、来源启停及可用动作归入 Gateway 本地方法组，复用阶段 4 transport；Provider 私有 observation 方法不直接向 UI 透传。Host dispatch、capability 和事件分发都限制 local-only，客户端自报类型不能授予权限。窗口/焦点/拖动/剪贴板仍由平台 adapter 实现。

动作先按真实能力提供打开来源和快速回复；不支持的来源不显示按钮。请求有稳定 ID、绑定目标及 revision；受理/成功/失败/结果未知分开，未知结果不自动重发。执行记录存储用已有 Host 基础按需增加，不改造全部数据库。真实 Agent 无可靠回复接口时只提供打开来源。

**涉及模块与原因**：Provider descriptor/能力合同与 Host discovery、三 Provider、codepet-observation、Pet projection、Gateway 的本地方法/事件、Pet UI；解决固定来源、原生解析位置和私有前端协议三个实际问题。

**验证/通过条件**：observation-only 不启动 Agent；退订观察不停止 Remote 任务；现有归并测试迁移后仍通过；源/账号隔离、旧流事件、撤回、重复动作与未知结果正确；远程无法调用或订阅本地 Pet；桌宠与顶部两视图一致，中文输入/焦点/透传/尺寸/激活目标通过 Tauri 验收。

协议能否保持兼容在本阶段具体 schema diff 后决定：不为整理预设升版；但减少必填项、增加封闭 DTO 字段或改握手若影响旧二进制，必须显式兼容入口/版本或同批最低 Host 要求，不能在旧版本下冒充无变化。一个来源只运行一条新旧观察链。

### 阶段 6：兼容路径清理与最终发布验收

**工作**：确认消费者已迁移后移除无调用的旧 v0 bridge 和独立 Pet 前端入口；不整目录删除旧 agent/activity 模块，不触碰任务谱系。根据实际 App 兼容变更提升 webcontent/native API 门槛，Host/内置 Provider/前端按同批版本验收，SDK 物料与实现一致。

**涉及模块与原因**：lib.rs 注册、runtime_gateway 兼容适配、前端 import、webcontent 校验、build/release workflow、作者指南和运行拓扑；避免保留多条无主链路及错误文档。

**验证/通过条件**：最小自动化检查通过后扩大到 Host/Provider 集成、Tauri 构建、手机回归和安装包检查；Mac/Windows 按实际支持范围验证。旧调用检索、资源白名单、离线 SDK 编译和真实运行证据齐全，剩余未验收项明确。每批回滚自身实现与配置，不删除已有业务数据，不把结果未知的动作重新执行。

## 推进规则与依赖

推荐主线：0 → 1 → 2 → 3 → 4 → 5 → 6。阶段 3 的纯物料 profile 可提前独立交付，阶段 1/2 不依赖大规模协议变更。每个阶段按可构建提交拆分，不能等最终阶段才发现导出、Tauri 枚举或手机编译失败。

每批固定四项：范围/行为说明、源码与附近测试、最小必要检查、验证记录与回滚方式。发现已有失败先区分基线；新故障未解决不进入下一阶段。修改后的首轮最小检查通过后，只有新改动或未解决问题才重复/扩大测试。

建议第一轮止于阶段 3，得到更清晰的通道设施、初始化与正确 SDK 包；第二轮推进阶段 4/5，最后清理。存储物理迁移、全量本地管理接口迁入 Gateway、common 物理合包和真实 IM 接入均不作为第一轮门槛。

## 测试计划

| 改动类型 | 首轮检查 | 扩大检查的时机 |
| --- | --- | --- |
| 文档/纯归属 | 本地链接、依赖检索、git diff --check | 不因只改文档运行全仓构建 |
| 通道设施 | 相关 Host 日志/session/ownership/LAN 测试 | 改装配生命周期时跑 RTC/热切换/撤销，实机网络另记 |
| 初始化 | 三 Provider 共用合同与异常测试、SDK mux/Host process_rpc | 改 runtime/生命周期时跑内置 Provider 集成及 Tauri 编译 |
| IDL/SDK | npm run protocol:check、npm run sdkgen:test、npm run providers:test | 导出包仓库外 Rust 编译；Gateway 受影响时加 TS/Dart；最终 bundle 离线验证 |
| Gateway/前端 | 相关 Vitest、adapter 合同、本地在线 guard 回归 | 新增事件/方法时验证所有实际枚举消费者与远程访问范围，跑手机既有用例 |
| 桌宠 | pet model/展示测试、相关 UI QA | 原生输入、窗口、穿透、拖动或激活变化必须实机验收 |

可复用现有 Rust入口：sdk/rust workspace 的 codepet-provider-sdk、Host process_rpc（含 mux_handshake_cancel_and_shutdown_use_the_production_sdk_runtime）、manager_gateway 的 codepet_usage_routes、remote_lan_listener 的 negotiated_channel_handover，以及 builtin_provider_integration。不要只运行源码扫描测试作为运行行为证据。

手机沿用其 controller/cache/DeviceSession 的既有测试，按实际改动运行 flutter analyze/test；本轮不修改手机分页/缓存规则。涉及最终发布时再运行完整构建、签名/资源校验，不能用 mock UI 通过代替安装包运行。

## 风险与控制

- 初始化抽取引入状态或资源泄漏：成功提交、失败清理、并发和重试测试；保留 bootstrap/业务 initialize/原生 initialize 的区别。
- 通道统一误用本地信任或让静默页面永久在线：调用上下文、请求 guard、网络拒绝和生命周期合同测试。
- 事件切换造成重复/缺口：原子 fence、cursor 重放、generation 和重挂测试；未知保持未知。
- Provider 能力拆分误启/误停 harness：观察与控制并发、退订/释放控制权测试。
- SDK 精简导致导出失败或削弱校验：源码完整 fixture 校验、发布输入独立校验、文件清单和最终包黑盒编译。
- 修改共用模型牵连任务谱系：不迁移其业务，保留现有类型兼容；被全仓编译覆盖，避免顺手删除。
- 数据/配置回滚重复副作用：分批备份托管配置，已有数据库与执行记录保留，结果未知不重发。

## 知识沉淀

本文为当前推进方案，旧完整迁移设想已被替换。Review 保留事实与当时测试结果。每阶段落地后仅更新相关协议、Provider、通道、桌宠或分发文档及复用规约，不把计划直接写成当前能力。

## 未知项与当前验证记录

外部旧接口消费者、Observation-only 合同对版本的具体影响、首个真实 IM 来源、真实 Agent 快速回复能力及 action 核对方式仍待各阶段验证。工期按阶段实际工作评估，不提前承诺天数。

本轮只重写计划和标注范围，执行 git status --short、已有源码/测试/文档读取、本地链接与 git diff --check。未修改实现、运行应用测试、构建、删除任务谱系或安装新包。上一轮 30 项测试通过仅作为当时基线。
