# SDK 公共边界与 App 分发内容评估

> 已合并为 [Observation 与 SDK 完整改造计划](observation-and-sdk-refactor-plan.md)。本文保留已安装 App 和构建链路的调查依据，实施方案以完整计划为准。

状态：2026-11-11 只读评估及文档记录，未修改 SDK、协议或打包逻辑。用户提出对外仅保留 common、gateway、provider 三类 SDK。

## 背景与证据

实际读取 `/Applications/Code Pet.app/Contents/Resources/provider-sdk`：包含 cp-sdk-gen、README、codepet-sdk.json、codepet-provider-sdk.json，以及 core/agent/provider/gateway/channel-lan 五组协议输入。protocol 共 126 个文件、249725 bytes，其中 fixtures 路径下文件含索引共 115 个、59314 bytes；还包含 `gateway/v1/webrtc-cpg1.md`。这是本机安装包快照，不等同于所有发布版本。

`scripts/stage_provider_plugins.mjs:stageProviderSdkResources` 逐目录递归复制上述五组协议，未过滤 fixture 或 Markdown；资源目录名仍为 provider-sdk，但 codepet-sdk.json 已将它声明为多目标工具包。该目录主要是代码生成器与 IDL 输入，不是一个同时导入全部协议的运行时库。

`tools/cp-sdk-gen/cp-sdk-gen.mjs` 可选择 provider、gateway、lan-channel，并只为选择目标及依赖生成 SDK。Provider 和 Gateway 输出依赖 core+agent，Rust Cargo 与 Dart pubspec 没有 LAN/WebRTC 依赖。对外生成 CLI 当前支持 Rust/Dart 的部分 role，仓库本身还存在 TypeScript 生成目标；两者不能混称相同支持范围。

`protocol/gateway/v1/manifest.json` 却仍写 framing=websocket-text、additionalFramings=[webrtc-cpg1]，说明业务 manifest 与具体通道描述仍有整理空间。LAN schema 定义 QR、pairing 和凭据等准入 DTO，并非完整 LAN 网络实现；WebRTC 文件是通道规范文档，并非被编译进 Provider 的 WebRTC 客户端。

## 判断

支持将 common / gateway / provider 作为三个面向使用者的公共 SDK 入口。此前 Observation 提案中的 plugin、observation 是内部协议职责划分，不要求每一层都发布一个独立 SDK；应优先避免暴露更多顶层包名。

| 公共 SDK | 职责 | 不应默认携带 |
| --- | --- | --- |
| common | 共享基础类型、错误、版本、资源身份、公共业务模型；可按基础/Agent/Observation 分模块组织 | 配对凭据、具体通道实现、宿主进程管理和测试数据 |
| gateway | 面向手机、App 前端、TUI 的 Host 业务访问：typed client、事件、错误与抽象 transport；server role 仅供需要的宿主实现者 | Provider 私有进程接口、强制 LAN/WebRTC 或发现配对依赖 |
| provider | 面向扩展作者的接入/采集/处理/动作实现接口；公共生命周期，Agent 与 Observation 能力分模块提供 | Gateway client、手机配对流程和无关通道数据 |

Provider 可以实现查询、处理、回复与 Agent 执行等扩展能力，不应仅理解为读取数据，也不应要求 Observation-only 实现 Agent harness。共享 Agent 模型保持一份定义；整合到 common 的公共入口不等于删除领域分层或将所有类型平铺进基础 Core。若物理合包产生过大依赖，应使用模块、feature 或重导出保持按需使用。

Gateway 支持不同调用方，靠统一业务协议和 transport contract 实现；本地 Tauri IPC、远程 WSS、未来其他传输分别适配。客户端只能调用其上下文允许的方法，统一 SDK 不等于把本地桌宠私有方法或 Provider 生命周期全部开放给手机。

## fixture 是什么

fixture 是可重复的协议测试样本，例如合法 provider.initialize 请求、响应、事件，以及缺字段/非法值的负例。它用于校验 schema、跨语言编解码和协议兼容，不是运行时业务数据、补丁或 fix。

现有生成器在构建模型时无条件调用 validateFixtures；各 manifest 的 fixtures 字段指向索引。因此当前工具从 App 资源生成代码会读取这些样本。不能仅删目录，否则导出会失败。但这种依赖来自当前生成流程，不代表运行时 SDK 必须携带测试数据。

建议 fixture 保留在仓库测试与可选的协议一致性测试包中。源码 CI 执行完整 schema+fixture 验证；正式分发使用经过验证的生成输入，代码生成保留结构/引用/能力校验。可把 fixture 索引移至测试配置，或区分源码与发布 manifest；不能直接让源码 CI 静默跳过缺失测试。

## LAN / WebRTC 放在哪里

业务 SDK 不默认依赖具体通道。实际使用局域网的手机仍需要发现、配对、证书验证及 WSS 建链；RTC 仍需要信令与 framing。这些代码及协议应保留在对应 Host/客户端 adapter，必要时另发可选连接组件，不删除实现，也不塞入 Provider 作者的默认 SDK。

建议将 Gateway manifest 中的具体 framing/profile 归入独立连接配置，业务层仅描述 JSON-RPC 及 transport contract；规范文档保留在协议/架构文档目录。通道协商与安全校验仍由实际 adapter 执行，移动文档不改变 wire 行为。

## 分发建议

App 工具资源使用与范围匹配的名称，例如 codepet-sdk；默认面向三个公共目标按需导出。Provider 输出仅含 provider+common，Gateway 输出仅含 gateway+common；common 可以独立导出。按语言选择可用目标，未实现语言/role 明确拒绝。

发布资源只携带生成所需 schema/manifest、生成器和目标使用说明，采用依赖闭包与文件白名单；fixture、设计提案、通道规范不因目录递归复制而自动进入默认分发。接入示例、协议验证样本和连接 adapter 可单独获取。App 运行资源与给扩展作者的开发资源应区分用途；是否仍随 App 离线携带完整生成器是后续分发决策。

## 涉及模块与验证

- `protocol/codegen.json`、生成器：统一公共入口与内部类型依赖，分开生成和测试输入。用缺失 fixture、负例拒绝、schema 引用错误测试验证 CI 未弱化。
- `cp-sdk-gen`、SDK Cargo/pubspec/package：按目标导出最小依赖，验证 common 独立、Provider/Gateway 独立编译，检查没有隐藏 path 依赖。
- `stage_provider_plugins.mjs`、Tauri resources 与资源元数据：显式分发白名单、目录命名和导出说明一致；通过 staging 测试及最终 App bundle 的离线生成/编译黑盒检查。
- Gateway 与 LAN/RTC adapter：修改协议描述归属时跑现有网络/配对和 RTC 回归，验证安全、分页、事件与请求语义一致。
- Observation 提案：按三个公共 SDK 入口组织公共生命周期与可选能力，不把内部拆包数量当成对外产品数量。

## 未知项与后续观察

common 是物理合包还是公共 facade、连接 adapter 是否独立发布、生成器是否默认随 App、现有扩展作者兼容需求尚未确认。此次发现主要是分发语义和边界问题；fixture 总体约 59 KB，不应把删除它描述为主要包体积优化。

本轮执行 git status、源码和附近测试阅读、已安装 App 资源统计及 git diff --check。未执行生成器、修改已安装 App 或运行应用测试。

## 构建产物与 methods.json 命名评估

用户进一步询问构建产出，并提出协议 manifest 应为 methods.json。以下为建议，尚未改文件名、CLI 或构建流程。

### 输入职责

当前协议 manifest 同时包含 methods、events、capabilities、error、transport、version 和 fixtures，不能仅改名后宣称它已经是纯方法表。建议拆分职责：

- `schema.json`：公共与业务类型定义。
- `methods.json`：方法名、方向、request/response ref、能力、幂等性、调度分类等方法契约。
- `events.json`：事件名、payload ref、方向、scope、交付语义；只有类型的 common 不需要空方法/事件表。
- 已有生成配置：包身份、版本、依赖、公开类型、语言和输出配置；避免在多个文件维护冲突的版本值。
- 测试配置：fixture 索引、正负例与一致性用例，构建校验时读取，不进入发布业务契约。
- 连接配置与 adapter 文档：具体 WebSocket/WebRTC framing 和 LAN admission，不进入公共方法表。

JSON-RPC 的请求/响应分类、协议错误与事件 envelope 等公共规则仍必须有可生成的权威定义，可由 common schema 与生成器协议描述承载；移除大 manifest 不能丢失这些约束。编解码器从同一规范化 IR 输出，不从各语言接口反向推断另一套契约。

方法和事件声明作为手写 IDL 输入，SDK 的接口、dispatcher、方法常量与对外索引由它们生成。对外若附带 methods/events 机器可读表，应是同一输入的发布副本或确定性规范化结果，不能再手写维护一份。

插件安装清单 `codepet-provider.json` 是另一类物料：包含 pluginId、可执行文件、参数、环境和实例配置，仍属于插件 manifest，不能改成 methods.json。SDK 产物的 sdk.json/package metadata 记录版本、依赖、角色、语言、输入 digest 和校验和，也不属于方法声明。该命名调整只针对协议 IDL 的职责拆分。

### 当前构建事实

`package.json` 的 build:bundle 执行 protocol:check → providers:stage → build。protocol:check 验证仓库中已生成的 SDK 新鲜度及测试，不负责在此步骤重新生成。providers:stage 构建 Provider 和平台对应的 cp-sdk-gen，复制协议输入整目录；最终 Tauri 将 resources/provider-sdk 打包。App 中存在生成器不等于构建时已产出三份干净的公共 SDK。

### 建议的构建流水线

1. 校验手写 IDL：schema、methods、events、引用、能力和依赖方向；执行仓库完整正负 fixture 验证。
2. 将公共入口解析成规范化 IR，按 common / gateway / provider、语言及 role 生成到干净临时输出目录。复用同一生成器，不另写面向打包的类型转换器。
3. 将生成代码与目标所需手写运行时、包描述和按目标生成的说明组装；依赖使用发布目录内合法相对路径或一致的包版本。按依赖闭包带 common，不复制全部协议层。
4. 独立验证输出：Rust cargo check、TypeScript 编译、Dart analyze，配合跨语言 fixture 与编解码测试。无编译目标声明不能自动当作已支持语言。
5. 产出源码 SDK 包与 sdk.json（版本、语言、角色、输入 digest、依赖、文件校验和）。共享同一输入 digest；fixture 和时间戳不作为业务协议版本来源。支持的 client/server role 分别有验证。
6. 统一 SDK staging 输出成为 App 与下载附件的输入，App 不再递归复制源协议目录。App 若继续提供离线 cp-sdk-gen，另装最小发布 IDL 和对应平台工具，并做最终 bundle 中的离线导出/编译验收。

产物组织示意：`dist/sdk/<language>/{common,gateway,provider}/`，语言源包通常无需按 OS 重复生成；cp-sdk-gen 这类可执行工具才按 OS/架构构建。实际包内路径需符合 Cargo/pub/npm 的依赖布局，可采用每份归档携带 common 或共享 workspace，不能把未打包的工作区绝对路径带给使用者。

如果继续提交 sdk/ 生成代码，CI 应用同一生成器验证源输出与干净构建结果一致；App 使用哪一组输出要显式规定，不能让源码检查和分发各自维护一条生成链。手动 protocol:generate 可继续服务开发，正式构建必须拒绝陈旧输入/输出。

### 变更范围与验证

需同步修改 codegen 配置读取、normalize IR、依赖/能力审计、fixture 测试入口、digest 计算、cp-sdk-gen 独立资源定位、README 与 staging 测试。生成 Rust/Dart/TS 的 wire 名称与字段可保持一致，IDL 文件改名本身不需要提升 wire 协议版本；生成器导出布局或输入格式有破坏性变化则需单独标识工具/物料格式版本。

验收包括缺失/重复方法和事件、错误 `$ref`、负例未被拒绝、fixture 未进入发布目录、独立 SDK 在仓库外编译，以及从最终 App 解出的离线工具与发布 SDK 输入 digest 一致。本次只完成静态评估与文档更新，未执行上述生成和编译。
