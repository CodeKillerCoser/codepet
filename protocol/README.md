# CodePet 协议 IDL

## 权威源与依赖

Provider 是插件与 Host 之间的协议；Gateway 是 UI、手机、TUI 等操作端与 Host 之间的协议。两段分别编码和解码。两者共享业务对象的结构，不共享进程生命周期，也不要求使用同一种通道或压缩方式。

权威输入位于 `protocol/v1/`，版本统一放在这一层：

```text
v1/
├── protocol.json                 # 共享对象与服务入口
├── common/                       # ID、时间、分页、错误和基础 RPC 信封
├── agent-domain/                 # Message、Conversation、Turn、Project、Approval 等
├── provider/
│   ├── protocol.json             # 模块文件登记和服务元数据
│   ├── sdk-manifest.json          # 对外 SDK 发布元数据
│   ├── README.md                 # 对外说明的维护源
│   ├── manifest.schema.json      # 插件安装清单
│   ├── transport/                # 三种 stdio profile、传输握手
│   └── <功能>/                   # Schema、methods.json、可选 events.json
├── gateway/
│   ├── protocol.json
│   ├── README.md
│   └── <功能>/                   # 包括独立的 pairing 模块
└── channel-control/              # 通道准备、切换、状态控制
```

common 不依赖上层；agent-domain 只依赖 common。Provider、Gateway 分别引用 common、agent-domain 及本层模块，不互相引用。共享对象含义与结构都相同时才复用。Host 面向客户端的聚合结果、快照游标、输入队列归 Gateway；插件目录、实例启动和运行时选择归 Provider。

`pet/v1` 与 `desktop/v0` 保留既有独立合同。本轮未迁移旧桌面调用，不把桌宠或任务谱系加入 Agent 共享对象。

## 按功能维护

Provider 包含 lifecycle、instance、runtime、conversation、project、turn、approval、subscription、usage。Gateway 包含 session、subscription、provider、project、conversation、input-queue、turn、approval、usage、pairing。

每个功能的 `messages.schema.json` 定义请求、结果、事件载荷和本模块辅助类型；`methods.json` 与 `events.json` 是本功能方法和事件的唯一登记。没有事件的模块不创建空事件文件。服务 `protocol.json` 只登记模块文件，不再手写汇总方法列表。模块之间以明确的相对 `$ref` 引用对象。

所有 Schema 的根、对象、字段、数组元素和联合分支都要有 description。字段含义和约束放 Schema，使用流程与生命周期放 README。方法名、方向、幂等性、能力关联、事件作用域与投递语义由登记文件定义。生成器聚合后检查重名、引用闭包、跨层依赖、能力关联、约束与 fixtures。

JSON-RPC 使用字符串请求 ID、标准数字错误码与业务错误信息；成功结果与错误互斥。基础信封来自 common，专用方法/事件联合从注册表派生。Gateway 的 trace metadata 和事件游标由本层 service 元数据定义。心跳值由 lifecycle/session 策略生成到运行时，不另手写常量。

## 通信生命周期

Host 按插件安装清单的 command、args、env 启动独立 Provider，组装选择的 stdio profile；需要传输协商的 profile 先完成传输握手，再调用 provider.initialize 注入身份、目录等上下文。随后进行描述、实例管理和业务调用；应用心跳独立处理，结束时调用 shutdown 并监管退出。三种 profile 的完整 wire 和限制定义在 Provider transport，使用说明见 Provider README。

Gateway 的配对归 `gateway/pairing`，不归 LAN。现有配对类型仍保留 LAN/TLS 字段和旧 SDK 包名，以保证兼容；其 HTTP 绑定记录在 pairing/bindings.json。它们不作为已认证业务 RPC 注册。当前文件覆盖原有 IDL 的配对对象与绑定，不声称已经把所有云端邀请/信令实现都迁成 IDL。新增跨通道字段需要另行设计与验证，不能只靠移动目录改变 wire。

完成通道建立和相应身份授权后，远程客户端调用 protocol.handshake，交换客户端身份、版本、Host 与 Provider 摘要；随后查询业务快照，通过 event.subscribe 从不透明 snapshotCursor 恢复事件，并维护 protocol.ping。桌面本地 IPC 使用 Host 的本地调用入口，不机械套用远程准入流程。

IPC、WebSocket、WebRTC 是 Gateway 的承载适配器，不需要每种通道都新增一套业务 Schema。LAN 发现、TLS 证书、RTC ICE/DTLS、分片和压缩属于通道规则；通道切换控制有自己的协议，不能混入业务 dispatcher。业务请求并发执行，控制消息和心跳不等待慢业务。

## 共享对象与同步语义

资源在客户端使用不透明 `providerId + nativeResourceId`；Host 与 Provider 使用完整实例路由，并校验跨资源实例一致性。客户端不能解析或拼接事件游标。快照边界在发起查询前捕获；客户端原样保存 snapshotCursor，按订阅结果衔接 replay 与实时事件。分页逐页返回，不在中间层聚合全量历史。

领域联合使用必填的单值 kind 判别，生成器拒绝歧义联合和开放结构。工具调用的输入、结果各只有一个完整载荷所有者；终态正文归完整 item，进行中正文通过 delta 投递。具体约束继续遵循 `knowledge/60-rules/` 中的协议分层、工具载荷所有权、分页与同步规约。

## 生成、构建和分发

`tools/protocol-codegen/modular-contract.mjs` 加载权威模块并组装视图；`generate.mjs` 根据 `codegen.json` 的包与 target 配置生成 Rust、TypeScript、Dart。Python target 尚未实现，选择时明确失败。手写代码负责业务、连接和编解码机制，不能成为第二份 DTO 或方法注册表。

`core/v1/schema.json`、`agent/v1/schema.json`、`provider/v1/{schema,manifest}.json`、`gateway/v1/{schema,manifest}.json` 以及 `channel/{lan,control}/v1/{schema,manifest}.json` 是生成的兼容视图，保留既有语言包、Host、桌面、手机的引用路径，不可直接修改。旧路径下的 fixtures 是回归样本。仓库默认生成必须读取 protocol/v1，不能退回兼容视图作为事实源。

对外导出仍为 `provider-sdk/v1/{sdk-manifest.json,README.md,protocol/}`；protocol 内分安装清单 Schema 与 sdk 通信生成输入。导出器复制所需依赖闭包、聚合功能模块、重写相对引用并生成源摘要。包内不包含 Gateway、配对、通道控制、fixtures、Host 源码或语言生成工具。App staging 使用同一导出器。内部 cp-sdk-gen 仍供开发者导出语言 SDK，不随 App 公开 SDK 分发。

```sh
npm run protocol:generate
npm run protocol:check
npm run sdk:export -- <空输出目录>
npm run providers:test
npm run sdkgen:test
```

协议检查覆盖迁移前后的 wire 结构摘要、模块重名、description、引用/依赖、默认值、能力关联、fixtures 和生成新鲜度。再以语言 SDK 测试及 Host 的真实 Provider 进程、Gateway、LAN/RTC 测试验证运行行为。内部代码生成工具不能代替独立 Provider 的跨进程互通验收。
