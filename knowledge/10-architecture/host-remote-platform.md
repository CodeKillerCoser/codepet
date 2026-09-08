# Host / Remote 双端架构与多进程扩展边界

## 背景

Code Pet 当前以远程控制与 Provider 插件扩展为产品主线。桌宠是本地附加体验，不能用桌宠事件管线概括整个系统。本页是跨仓库架构说明，不新增中央知识索引，也不改变现有协议。

## 证据与阅读范围

本次核对本工作区的运行拓扑、Provider Host、协议分层、Gateway/准入边界、连接心跳、离线执行、最近会话、双向配对文档，以及 `tools/cp-sdk-gen/README.md`。对照 `frontend/App.svelte`、Host 内置 Provider 集成测试和 Provider manifest/源码检查模块归属。

Remote 以 [f8328b0](https://github.com/CodeKillerCoser/codepet-remote/tree/f8328b04815ec8a281196fec32bd7f3900467783) 为阅读快照，核对 README、`docs/architecture.md`、最近会话、地址恢复、Provider 图标、资源传输提案，以及 `lib/gateway/gateway_client.dart` 和架构层次测试。Remote 仓库只读，未修改其文件。

## 架构与职责

| 边界 | 责任 | 关联模块及原因 |
| --- | --- | --- |
| Remote 应用 | 设备会话、项目/会话投影、发送、审批与展示 | Remote 的 `lib/features`、`lib/application`、`lib/core` 分离界面、用例与领域，避免界面直接依赖原生 Agent |
| 网络准入与通道 | 发现候选、确认配对、凭据、TLS/WSS | Host `crates/codepet-host/src/remote/`；Remote 的 admission/channel/discovery/security 管理连接与信任 |
| Gateway 业务入口 | Provider 枚举、统一业务路由、快照和事件 | Host Gateway 与生成 Gateway SDK；不负责读取原生工具 DTO，也不把配对放进业务协议 |
| Provider Host | 插件发现、独立进程、实例生命周期和请求路由 | Host `providers/`；Tauri 组合并管理退出，Host 是桌面主进程内的模块，不是额外的网络服务进程 |
| Provider 插件 | 原生协议适配、能力声明、标准事件 | `crates/providers/` 和第三方插件；隔离 Codex、Claude、OpenCode 等工具实现差异 |
| Agent runtime | 在电脑上执行实际任务 | Provider 管理的本机 App Server、CLI 或 Server 子进程；底层工具由用户安装配置 |
| 本地桌宠 | 本地观察与提醒 | Pet/Hook/Companion 链路独立；Provider 事件不直接流入桌宠列表 |

## 多进程模型

桌面应用不是把所有 Agent 嵌入同一进程运行。Tauri/Rust 主进程承载 Host 的 Gateway、插件管理与本地服务，主窗口和桌宠窗口是界面入口；WebView 的系统辅助进程数量由平台决定，不能按窗口数推断。

Host 按插件 manifest 启动独立 Provider 可执行文件。每个插件有独立 stdin/stdout/stderr 与 RPC 状态，可以配置多个运行实例；“插件进程”与“运行实例”不是一一对应。Server 模式的实例共享一个 Harness Server 服务多个会话，不能画成每个会话固定启动一个 Provider 或 Server。

Provider 再管理底层 Agent runtime 子进程。请求按身份和实例路由，事件经 Gateway 返回 Remote。退出、EOF、心跳失联和故障清理由进程生命周期管理约束；普通手机离线不应与 Host 失联混为一谈。

独立进程有利于故障隔离与资源清理，但当前不代表操作系统权限沙箱、插件签名验证或资源配额。manifest 是受信任的本地配置。

## 技术选型与取舍

跨平台是核心选型因素：Web 桌面界面、Flutter 客户端与 Rust Host 尽量共享业务实现，系统相关窗口、电源、网络和进程能力由平台适配承接。前端通过 Tauri IPC 与 Rust 主进程交互，不能在图中遗漏前端，也不能把系统托管的 WebView 误标为 Rust 主进程内模块。共享框架是扩展基础，实际交付范围仍需按平台验证。

| 技术 | 当前用途与设计收益 | 代价或边界 |
| --- | --- | --- |
| Flutter / Dart | 共享跨平台客户端业务与交互，纯 Dart 用例层经生成的 Gateway client 接入统一协议 | 当前实际工程以 Android 为主，不能把框架跨平台能力当成 iOS 已交付 |
| Tauri 2 / Svelte 5 / Vite | 跨桌面平台复用 Web 界面，以 Tauri 接入原生窗口与系统能力 | 原生行为与系统依赖需按平台验证 |
| Rust / Tokio | 共享跨平台 Host/Provider 逻辑与异步进程 I/O，支持有界队列、并发 RPC 和有序退出 | 多进程与多实例带来生命周期、背压和路由校验成本 |
| JSON Schema + manifest + cp-sdk-gen | 统一模型、方法、事件、能力和版本，生成 SDK，减少两端手写协议漂移 | 新字段/方法需同步生成 SDK 并验证 Host/Remote 兼容性 |
| LAN HTTPS / WSS | 当前发现配对和远程请求/事件链路，证书 pin 与凭据校验在通道侧完成 | 局域网隔离、地址变化与移动端生命周期需要真实设备验证 |
| Provider stdio IPC | Host 与独立插件交换 JSON-RPC，SDK 管理复用、分片和流控 | 不属于 Remote 网络 channel，插件业务不能绕过 SDK 自写不兼容帧 |

这些设计收益是基于模块边界的解释，不是未经测量的性能承诺。

## 扩展契约

标准能力的扩展路径是：实现 Provider 协议与生命周期、声明 capability、发布标准事件，提供 manifest 与可执行文件，安装到 Host 插件目录。Host 加载后 Remote 经 Gateway 枚举 Provider，并按能力呈现现有操作，不需要认识工具的原生 DTO。

新增协议尚未表达的业务仍需更新 schema/manifest、生成 SDK、实现 Host 映射与 Remote 展示。协议输入可以面向多语言，当前生成器的 Provider server target 是 Rust；不要声称已有任意语言 Provider SDK。具体安装、SDK 导出和纵向验证见 [Provider 指南](../../tools/cp-sdk-gen/README.md)。

## 文档差异与当前边界

- Remote 阅读快照的架构说明仍写最后客户端断开会终止活动任务；当前 Host 的 [离线执行](host-awake-and-offline-execution.md) 已改为保留未结束 turn、待审批与执行中请求，空闲后才回收。必须区分部署版本；不据此承诺 Host 重启或系统睡眠恢复。
- 部分旧资料仍描述四段 Gateway wire 身份；当前 [协议分层](protocol-layers-and-device-routing.md) 区分 Gateway 的两段 opaque 身份与 Provider 的四段实例路由。README 不固化底层字段数量。
- [WebRTC](remote-webrtc-datachannel-proposal.md) 和 Remote 资源 URI/文件传输文档明确属于设计提案。当前架构图展示 LAN HTTPS/WSS，不将提案当作已经交付。

## 风险与验证路径

- 跨版本描述漂移：协议 freshness 检查加 Host/Remote 同版本联调，核对生成 SDK digest。
- 第三方能力过度承诺：用独立插件验证 discover/describe、发送、事件和不支持能力的 UI，不能仅看 manifest 声明。
- 进程边界误画或隔离夸大：对照 Host 进程管理和三 Provider 纵向测试，人工核对图中 Gateway/Provider Host 同属桌面主进程。
- 文档上手步骤偏离产品：按连接页实际标签与 Remote 配对用例核对；完整体验仍需双机配对、发送、审批、断线重连验收。

本次仅修改文档与配图，执行文档链接/格式和图像人工检查，不重复运行应用测试；双机、平台电源与真实 Agent runtime 验证未在本次执行。已有验证证据以各领域文档为准。
