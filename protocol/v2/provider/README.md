# Provider SDK v2

本 SDK 定义 Code Pet Host 与独立 Provider 进程之间的接入合同。Provider 可以用 TS、Python、Rust 等语言实现，也可以打包为自包含二进制。接入只需要本 SDK，无需 Host 源码或特定语言的桥接程序。

README 说明如何使用 SDK、各模块的职责、通信生命周期和字节协议。对象字段、必填项、取值范围和字段含义统一以 JSON Schema 及其中的 `description` 为准，本文不重复列举。

## SDK 的组织与使用

整个 SDK 按版本发布，各模块共用这一版本目录：

```text
provider-sdk/
└── v2/
    ├── sdk-manifest.json
    ├── README.md
    └── protocol/
        ├── manifest.schema.json          # 插件安装清单规范，手工填写
        └── sdk/                          # 通信 SDK 的生成输入
            ├── transport/
            │   ├── profiles.json
            │   └── handshake.schema.json
            ├── rpc/envelope.schema.json
            ├── common/types.schema.json
            ├── agent-domain/domain.schema.json
            └── provider/
                ├── lifecycle.schema.json
                ├── messages.schema.json
                ├── methods.json
                └── events.json
```

| 文件或模块 | 用途 |
| --- | --- |
| [sdk-manifest.json](sdk-manifest.json) | 描述当前 SDK 的版本、入口和文件位置。它是 SDK 的发布清单 |
| [manifest.schema.json](protocol/manifest.schema.json) | 定义插件包的身份、名称、图标、启动命令、参数、环境、通信 profile 和实例配置。它描述整个插件包，不只是二进制文件 |
| [sdk/transport/profiles.json](protocol/sdk/transport/profiles.json) | 解释三个 profile ID 各自采用的定界、压缩、多路复用和尺寸限制 |
| [sdk/transport/handshake.schema.json](protocol/sdk/transport/handshake.schema.json) | 定义 mux 传输握手的 HELLO、SELECT 载荷及接收资源限制 |
| [sdk/rpc/envelope.schema.json](protocol/sdk/rpc/envelope.schema.json) | 定义 JSON-RPC 请求、成功响应、错误响应和事件的外层结构 |
| [sdk/common/types.schema.json](protocol/sdk/common/types.schema.json) | 公共身份、路由、版本、分页、错误和连接快照 |
| [sdk/agent-domain/domain.schema.json](protocol/sdk/agent-domain/domain.schema.json) | 会话、消息内容、项目、任务、审批、模型和用量等领域对象 |
| [sdk/provider/lifecycle.schema.json](protocol/sdk/provider/lifecycle.schema.json) | Initialize、Describe、Ping、Shutdown 的参数与结果，以及插件描述和初始化目录 |
| [sdk/provider/messages.schema.json](protocol/sdk/provider/messages.schema.json) | 其余 IPC 方法的参数、结果、事件载荷及实例能力对象 |
| [sdk/provider/methods.json](protocol/sdk/provider/methods.json)、[events.json](protocol/sdk/provider/events.json) | 注册可调用的方法和可发布的事件，关联对应 Schema，并声明方向、幂等性、调度或交付规则 |

先从 SDK 清单读取入口。类型、接口和校验器可使用标准 JSON Schema 工具生成；方法和事件从注册表读取。相对 `$ref` 按所在文件位置解析，因此使用和分发时应保留整个版本目录。SDK 的目录版本、插件 Manifest 格式版本、传输版本和业务协议版本分别解释，不要求数字相同。

## 自动代码生成的范围

`protocol/sdk/` 是通信 SDK 的生成输入；`protocol/manifest.schema.json` 只用于填写和校验插件安装清单，**不纳入通信 SDK 代码生成**。`sdk-manifest.json` 是 SDK 发布元信息，也不是业务代码生成输入。

| 输入 | 可生成的内容 | 仍需实现的内容 |
| --- | --- | --- |
| common、agent-domain、provider 下的 Schema | DTO、类型/接口、枚举、联合类型、序列化结构或运行时校验器，取决于语言工具能力 | Provider 的业务行为、状态存储、原生 Agent 适配 |
| rpc/envelope.schema.json | 请求、响应和事件的外层类型与校验器 | 请求 ID 关联、超时、取消和消息分发 |
| transport/handshake.schema.json | HELLO、SELECT、资源限制的 JSON 载荷类型与校验器 | CPMX 字节头、Yamux、GRANT/ACK、压缩与流控 |
| provider/methods.json、events.json | 配合方法/事件生成器可生成方法签名、处理器接口、分发或事件封装的骨架 | 每个处理器的业务实现；普通 JSON Schema 类型生成器不会自行生成这些服务骨架 |
| transport/profiles.json | 作为传输选择及配置依据；它是协议描述数据，不是 JSON Schema | 按 profile 组织读写流程，或使用兼容的传输库 |

本包提供生成输入，不内置通用的跨语言代码生成器，也不要求通过 Rust bridge 接入。作者可选择目标语言的 JSON Schema 工具，保留相对引用以生成类型和校验器；需要方法骨架时再消费 methods/events 注册表。生成 DTO 不等于生成了完整可运行的 Provider 或通信运行时。

实际开发时，先选择 profile，生成所需类型/校验器，实现相应读写和生命周期方法，再实现需要提供的业务能力。插件自己的 `codepet-provider.json` 由作者按安装清单规范填写，command、依赖和资源位置由最终构建产物决定，不能从业务 DTO 自动推导。

## 最终交付、导入与启动

最终交付物是一个**可安装的 Provider 运行包**。推荐独立设置交付目录，不把整个开发项目、构建缓存或 Provider SDK 包放进去：

```text
my-provider/
├── codepet-provider.json       # 必须：Host 的导入入口
├── bin/
│   └── my-provider             # 示例：已构建的可执行程序
└── resources/                  # 可选：运行时实际需要的资源
```

示例安装清单如下；字段含义与校验约束见 [manifest.schema.json](protocol/manifest.schema.json)：

```json
{
  "manifestVersion": 2,
  "pluginId": "example.my-provider",
  "displayName": "示例 Provider",
  "command": "./bin/my-provider",
  "args": [],
  "env": {},
  "transport": "stdio-jsonrpc-v1",
  "instances": [
    {
      "instanceId": "my-agent",
      "instanceKind": "my-agent",
      "displayName": "示例实例",
      "settings": {}
    }
  ]
}
```

只有 `codepet-provider.json` 的入口文件名固定；bin/resources 是推荐组织，不是 Host 强制的目录名。可执行程序须匹配目标平台并具有执行权限；Windows 程序可使用 `.exe` 并在 command 中写实际路径。单文件 TS/JS bundle 或 Python 入口也可直接放在包根目录，通过 command 指定语言运行时、args 指定脚本。

交付目录必须包含程序运行时需要的文件：打包后的代码、未内嵌的依赖和资源。依赖若没有打入单文件产物，就需要随包提供可运行的文件布局；Node/Python 等外部运行时则需在目标机器预先安装，并能被 command 找到。Schema 若仅用于构建时生成代码，无需随插件交付；插件若在运行时读取 Schema 做校验，则须把所需文件及其引用一并作为自己的资源交付。

**在 App 中选择导入的是 `my-provider/codepet-provider.json`，不是 SDK 的 Schema、sdk-manifest.json、可执行程序或 ZIP 文件。** 当前 Host 会复制所选 JSON 的整个父目录，再按安装后的 command/args 启动程序。因此 UI 只选择一个 JSON，并不表示程序和资源都已包含在该 JSON 中，也不表示只复制这一份 JSON。

发布时可以将整个 my-provider 目录压缩分发；使用者先解压，再选择其中的 codepet-provider.json。若清单只引用目标机器上已经安装好的外部程序且不依赖包内资源，交付目录可以只有这份 JSON；这是依赖本机环境的接入配置，不是可独立迁移的完整运行包。Host 不会根据 Manifest 自动下载程序、编译源码或安装依赖。

包中只允许普通文件和目录，不允许符号链接或特殊文件；重复插件 ID 不覆盖已有安装。SDK 的 sdk-manifest.json 不是插件启动清单。

Host 按插件 Manifest 的 `command`、`args`、`env` 直接启动进程，不隐式经过 shell，也不展开变量、管道或重定向。参数保持原样。包内相对程序路径相对于安装目录解析；没有路径的程序名优先匹配包内文件，否则通过 PATH 查找。包外绝对程序是环境依赖，不随导入复制。

进程启动工作目录为安装包根目录，会话工作区与它独立。

Host 根据 Manifest 的 `transport` 选择通信方式，并注入 `CODEPET_PROVIDER_TRANSPORT` 和 `CODEPET_PROVIDER_PACKAGE_DIRECTORY`。插件应读取所选 profile，遇到不支持的值即启动失败，不自动探测或降级。环境项不得覆盖这些 Host 保留键，也不得声明内部键 `CODEPET_PROVIDER_EXTERNAL_COMMAND`。新插件使用 Manifest v2 的 command/transport；旧 v1/executable 的兼容行为保持，缺省传输为 mux。

## Host 与 Provider 的交互生命周期

Host 管理插件进程的启动、路由、请求关联、超时和退出。Provider 管理自己的业务、任务和数据。双方通过子进程管道交换序列化数据，不共享 Host 内存对象、数据库连接或私有服务接口。

```text
Host                                        Provider 子进程
持有子进程 stdin 写端  ── 请求字节 ────────→ stdin
持有子进程 stdout 读端 ←─ 响应和事件字节 ─── stdout
日志采集              ←─ 日志文本 ──────── stderr
```

一次连接按以下生命周期推进：

1. **启动进程、准备传输。** Host 解析插件 Manifest 并启动程序。JSONL/zstd 直接进入业务通信；mux 先完成传输握手，再建立 Yamux 通道。
2. **Initialize。** Host 发送 `provider.initialize`，交换身份和业务版本范围，并按配置传入 Provider 数据、日志和数据库路径。Provider 校验并准备资源，成功后返回选定业务版本和插件描述；Host 校验版本及插件 ID。失败不能留下半初始化状态。
3. **描述与实例准备。** Host 调用 `provider.describe` 核对描述，并按配置创建、启动实例、读取能力。实例路由由 Host 指定，Provider 使用收到的身份。进程启动成功或 Initialize 成功，不等于所有实例已经可用。
4. **业务通信与心跳。** Host 可以并发发请求，Provider 按请求 ID 回应并主动发布事件；心跳在此期间独立进行。不同 Provider 使用独立连接和处理状态，一个插件的慢操作不应阻塞其他插件。
5. **退出与清理。** Host 发 `provider.shutdown`；Provider 清理自己拥有的工作，响应并将输出 flush 后退出。EOF 或传输失败也需清理自建子进程。Host 管理退出时限。

### 传输握手与业务初始化

**transport handshake 解决字节通道如何建立；provider lifecycle 管理插件的初始化、存活和退出。** 当前只有 mux 有显式 CPMX 传输握手，用于协商传输版本、必选能力和资源限制。Initialize、Describe、Ping、Shutdown 则是三种 profile 共用的 JSON-RPC 生命周期方法。

```text
JSONL / zstd：进程启动 → 按固定 profile 收发 → Initialize → Describe → 业务/心跳 → Shutdown
mux         ：进程启动 → CPMX 握手 → Yamux → Initialize → Describe → 业务/心跳 → Shutdown
```

Initialize 选择的是业务协议版本，CPMX 选择的是传输版本。传输握手成功后，Initialize 仍可能因业务版本不兼容或资源初始化失败而拒绝。心跳不是重复握手，也不是 Yamux 自身的传输 Ping。

Initialize 注入的是路径，不是数据库连接对象。Provider 负责其自有数据库的结构和事务，不依赖 Host 内部数据库结构。包目录用于资源，数据目录用于持久数据，会话工作区独立管理。字段与路径约束见 lifecycle Schema。

### 请求、事件与异步任务

请求和响应用 JSON-RPC `id` 关联。多个请求在途时，响应可以乱序，不能根据 stdout 消息到达顺序推断对应请求。成功响应的 `result` 必须按原请求方法在 methods.json 中的 response 引用校验；错误响应使用 envelope 中的 error 结构。外层数字错误码与业务字符串错误码分别解释。

Provider 主动发布 events.json 中声明的事件，事件的外层也使用 method/params，但不带 id，也没有 JSON-RPC 成功响应。响应与事件共用 stdout；接收端先完成传输定界，再识别消息类别。日志只能写 stderr。

以一次任务为例，Host 发 `turn.start`，Provider 返回受理结果，并通过输出增量、完整条目和任务状态事件持续报告进展。响应和首条事件可能交错到达，调用方应按资源 ID 关联。受理不等于完成，RPC 超时也不撤销已经受理的任务。

JSON-RPC id 关联一次交换，clientRequestId 用于业务幂等；重复的已受理工作不能再次执行。流式 delta 与完整条目保持 itemId/contentId 一致，完整历史可通过 `conversation.get` 获取。任务取消使用 `turn.interrupt`，断开 RPC 等待或远程界面连接不能代替业务中断。具体资源 ID 形态、输入和事件载荷见 common、agent-domain 和 provider Schema。

### 心跳与连接状态

Host 在插件进入 Ready 后运行 `provider.ping`，每个 Provider 同时最多一笔心跳在途。正常交换完成后等待 {{documentation.heartbeatIntervalSeconds}} 秒；客户端连接快照或实例状态变化可以提前唤醒下一次交换。

Provider 在同一进程内保持 hostSessionId 不变，要求 sequence 严格递增、clients.revision 不回退；revision 不变时客户端快照内容及顺序必须一致。全部校验成功后才更新本地快照，响应回显本次 sequence 和 clientsRevision。实例列表可以变化，不受 clients.revision 的不变约束。进程重启后重新初始化心跳状态。

Host 只接受回显匹配的 pong。自上次有效 pong 起达到 {{documentation.heartbeatTimeoutSeconds}} 秒后，失败交换会使连接标为 Offline；后续有效 pong 可恢复 Online。实际更新在交换结束时发生，单次 RPC 超时不等于立即离线。心跳只报告通信存活和连接存在性，不代表任务活跃，也不据此启动、停止 Harness 或终止任务。

## 通信协议

三个 profile 都走 Provider 的 stdin/stdout，使用同一套业务方法与数据结构。全部 JSON 为 UTF-8，字段与方法名区分大小写，JSON-RPC 版本固定为 `2.0`，请求 ID 使用字符串，不定义批量数组。以下多字节二进制整数均为无符号大端整数。

profile 是完整协议组合。zstd 提供压缩，Yamux 提供流复用，均不提供加密；当前合同没有额外网络监听、gRPC 或 TLS。各 profile 的固定容量限制见 profiles.json；mux 协商参数和允许范围见 handshake Schema。

### stdio-jsonrpc-v1：按行 JSON-RPC

每条记录是一个 JSON 对象，后接 LF 字节 `0x0a`。JSON 字符串内的换行正常转义。接收端只按 LF 字节定界，不能按 U+2028 等 Unicode 分隔符切行；应保留半条消息的字节，完整后再解析，并处理一次读取到多条记录的情况。

该 profile 没有压缩、CPRF 或 CPMX。第一条业务报文就是 Initialize。不同请求可以并发执行、乱序响应，但同方向的完整记录必须依次写入，不能交错字节。超长记录、无效 JSON 和 EOF 时未完成的记录不能视为正常消息。

### stdio-codepet-zstd-v1：分帧 JSON-RPC 与压缩

每条消息先序列化 JSON、独立 zstd 压缩，再添加 CPRF 头：

```text
CPRF magic[4] | version:u8=1 | encoding:u8=1 | payloadLength:u32 | payload
```

payloadLength 是压缩体字节数，不含 10 字节头。payload 表示一个完整 JSON-RPC 对象。此 profile 仅接受 encoding=1，每条消息独立压缩，不使用跨消息字典或连接级连续压缩流；接收端不依赖具体压缩等级。

接收端检查头部及声明长度后再分配，解压时限制输出和工作窗口。首条 Initialize 与后续请求、响应和事件使用同样的帧，没有额外传输握手。记录按顺序完整传输，业务执行仍可并发。损坏的定界或压缩使本连接失败，正常业务错误通过 RPC 返回。

### stdio-codepet-mux-v1：多路复用与压缩

mux 分为传输握手、Yamux 流和流内消息三个层次。

**传输握手。** 双方在 Yamux 之前依次交换 CPMX 报文：

```text
CPMX magic[4] | version:u8=1 | phase:u8 | jsonLength:u16 | JSON

Host     → Provider：phase=1 HELLO   （ProviderTransportHello）
Provider → Host    ：phase=2 SELECT  （ProviderTransportSelection）
Host     → Provider：phase=3 CONFIRM （JSON 数字 1）
Provider → Host    ：phase=4 READY   （JSON 数字 1）
```

JSON 本体最大 {{wire.bootstrap.maxJsonBytes}} 字节，完整握手需在 {{documentation.bootstrapTimeoutSeconds}} 秒内完成。HELLO/SELECT 的字段与限制见 handshake.schema.json。各方 receive 表示自己能接收的上限，约束对方发送，两个方向分别使用各自接收配置。版本、阶段、能力或限制不兼容时失败，不降级。

mux v1 必选能力是 {{documentation.features}}，必须完整且无重复。`yamux-1` 是 Code Pet 能力标识；Yamux 的 wire version 仍为 0。

**Yamux 流。** READY 后使用标准 Yamux 组件收发。Host 为 client，主动流 ID 为奇数；Provider 为 server，主动流 ID 为偶数。Yamux 使用 12 字节物理头、256 KiB 初始每流窗口；DATA 分片不得超过对方声明的上限。组件负责物理流拆分、重组和流控，但不负责 Code Pet 消息定界、解压或 JSON 解析。等待某个业务流时仍须推动整个 Yamux 连接。

**流内消息。** 一个请求/响应对使用一个双向流，Provider 另开流发布事件。每个方向先发送消息 envelope：

```text
version:u8=1 | class:u8 | encodedLength:u32 | decodedLength:u32
```

class 的值为 0=normal、1=small、2=control。encodedLength 是后续 CPRF 整帧字节数，decodedLength 是未压缩 UTF-8 JSON 的准确字节数。接收方验证并预留容量后回复单字节 GRANT=1；发送方收到后才发送恰好 encodedLength 字节：

```text
CPRF magic[4] | version:u8=1 | encoding:u8 | payloadLength:u32 | payload
```

此处 encoding 允许 0=raw JSON、1=zstd JSON。实际解码长度必须等于 decodedLength。压缩策略可以按大小和收益自适应，固定 zstd level 1 也能互通；等级不是 wire 字段。编码和解码尺寸都必须满足所用通道的上限，压缩变小不能绕过解码限制。

methods.json 中 dispatchLane=control 的请求及响应使用 control 预留资源。其他请求、响应和事件按自身大小选择 small 或 normal。**非 control 响应不能继承小请求的 class：小查询可能返回大历史。** 容量占用持续到消息处理结束，进入业务队列不等于释放额度。

响应在原流反方向使用自己的 envelope → GRANT → CPRF 交换，完成后 FIN；不能提前关闭响应方向。单流 RST 取消等待，不撤销已受理的业务修改。超限响应应在发送超限 envelope 前改为同请求 ID 的小型 RPC 错误。

事件流在 Host 接收交付后返回单字节 ACK=2；Provider 等待 ACK 后再发布下一事件，以保持顺序，随后结束该事件流。ACK 是传输交付确认，不表示持久化提交，也不是 JSON-RPC 响应。

三个 profile 都需要独立的消息定界和请求关联。无 mux 的两种 profile 仍可能因大记录占用物理管道而产生等待；mux 则通过独立流和 control 额度保护心跳等控制消息。

## 业务能力与运行时依赖

Provider 通过实例能力声明可用业务方法；对象、输入、选择与事件的具体含义见对应 Schema 和注册表。 能力目录与实际 selection 必须一致：create/acquire/turn 返回同一实例已生效的选择，声明了目录的维度使用其中存在且未禁用的选项，有默认值时返回实际默认选择；未声明的维度不携带值。目录变化应更新能力 revision，错误选择应明确拒绝。仅按 JSON Schema 校验一个对象，不能替代这类跨对象一致性校验。宿主、插件包、实例与原生 Harness 分别具有自己的生命周期，不能把“插件在线”直接解释为“所有实例可用”。

如果需要 Host 帮助选择外部 Harness，`runtime.getInstalled` 返回库存和当前选择，selectionRequired 缺省为 true。自行管理依赖的插件可显式返回 selectionRequired=false，此时清单应为空、selected 为 null、扫描不在进行且没有 scanError。App 显示“由 Provider 管理，无需选择”。这只表明选择职责，不保证依赖已安装；真实启动失败仍由实例诊断报告。


## 业务协议版本与前向兼容

Initialize 请求携带 `protocol: {version, minVersion}`，表示本端连续支持区间 `[minVersion, version]`，两者均为正整数。响应携带 Provider 自己的区间和 `selectedVersion`；选择 `min(host.version, provider.version)`，并要求不低于两端 minVersion 的较大值。Host 复核选择结果及插件描述中的区间；无交集即拒绝业务连接，不自动重试写入或降级传输。本版初始区间为 2/2，旧版 v1 严格 codec 不在此兼容承诺内。

新增可忽略字段、枚举、联合分支或方法时提高 version，保持 minVersion 的前提是保留既有语义并通过兼容测试。接收方忽略未知对象字段，保存未知枚举/联合原值；未知能力不调用，未知事件在验证信封后跳过。已知分支字段损坏仍拒绝。未知状态不能视为空闲、完成或授权，未知请求方法返回 -32601，未知请求选项应明确报错。新功能需通过相应能力声明门控，不能仅以版本号推断能力。

破坏结构或语义且无法维持旧行为时提高 minVersion。仅实现已有方法更新能力声明；模型/选项目录变化使用 capabilityRevision。业务区间独立于 SDK 发行号、Manifest 格式以及三个仍为 v1 的传输 profile。

Schema 的 enum/oneOf 列举本版认识的值和分支。普通 Schema 校验器默认是已知结构校验器；接收 codec 需要先识别判别字段，对未知值按上述兼容规则保留，对已知值继续按对应 Schema 校验。不能将整个 schema 校验失败统一视为未知内容。插件接收命令时，应明确拒绝不支持的选择。
