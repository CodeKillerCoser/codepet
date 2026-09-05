# CodePet Provider SDK 与接入指南

> 当前 `cp-sdk-gen` 是统一协议编译器，不只生成 Provider SDK。Provider 教程保留在本文后半；Gateway 和 LAN admission 使用下面的 role/package 命令。

## 0. 统一生成入口

```sh
# 独立 Provider 进程：Rust server trait + stdio runtime
cp-sdk-gen --package provider --role server --lang rust --output ./provider-sdk

# Remote 或其他 Gateway 接入端：Dart typed client
cp-sdk-gen --package gateway --role client --lang dart --output ./gateway-client-sdk

# 可开放的 Gateway 实现：Rust server trait + dispatcher
cp-sdk-gen --package gateway --role server --lang rust --output ./gateway-server-sdk

# LAN pairing/admission DTO
cp-sdk-gen --package lan-channel --role models --lang dart --output ./lan-sdk
```

Gateway Rust 还支持 `--role client` 和 `--role both`。生成目录包含 Core 依赖、目标 package、README 和记录 package/role/schema digest 的 `cp-sdk-gen.lock.json`。`--check` 用相同参数检查已生成目录是否漂移。

App Resources 使用 canonical 协议布局：

```text
provider-sdk/
├── cp-sdk-gen
├── codepet-sdk.json
├── README.md
└── protocol/
    ├── core/v1/
    ├── provider/v1/
    ├── gateway/v1/
    └── channel/lan/v1/
```

JSON Schema 定义 DTO 与约束；相邻 manifest 定义 method/event、方向、幂等性、capability 与 transport。两者缺一不可生成 service client/server。Gateway client 生成 typed 方法包装；Gateway server 生成 trait/dispatcher，trait 业务实现由接入者手写。channel（WSS、localhost 或未来 WebRTC）只实现 transport contract。

这份文档随 Code Pet App 和 `cp-sdk-gen` 生成结果一起分发，面向开发独立
Provider 插件的作者。Provider 是由 Code Pet Host 启动的独立进程，业务实现只面对
生成的 JSON-RPC 强类型接口；SDK Runtime 会把 JSON payload 自动封装为 stdin/stdout
Provider Frame V1。Provider 不需要感知长度前缀、raw/zstd、尺寸检查，也不需要依赖
Tauri、Code Pet Host 或 Desktop 私有 IPC。

## 1. 分发内容

macOS App 中的入口如下；Windows 使用相同资源结构，生成器文件名为
`cp-sdk-gen.exe`。

```text
Code Pet.app/Contents/Resources/provider-sdk/
├── cp-sdk-gen
├── codepet-sdk.json
├── codepet-provider-sdk.json
├── README.md
└── protocol/
    ├── core/v1/
    ├── provider/v1/
    ├── gateway/v1/
    └── channel/lan/v1/
```

- `protocol/core/v1`：跨协议共享的 ID、route、错误和分页等基础类型。
- `protocol/provider/v1`：Provider JSON-RPC 方法、事件、模型和 fixtures。
- `protocol/gateway/v1`：Gateway JSON-RPC 业务方法、事件和路由模型。
- `protocol/channel/lan/v1`：LAN pairing/admission 与凭据撤销 DTO。
- `codepet-sdk.json`：当前 App 携带的协议包、role、资源路径和生成器版本。
- `codepet-provider-sdk.json`：为旧 Provider 接入流程保留的兼容索引。
- `cp-sdk-gen`：导出与当前 App 协议严格匹配的 typed SDK。

资源目录保留 canonical layer-first 布局，各 schema 的相对 `$ref` 可从
App Resources 中直接解析，不需要另一份重定位协议。

## 2. cp-sdk-gen 是什么

`cp-sdk-gen` 是 JavaScript 编写的完整协议代码生成器，源码入口是
`tools/cp-sdk-gen/cp-sdk-gen.mjs`，复用 `tools/protocol-codegen` 的 schema 校验、
normalized typed IR 和语言 emitter。它在运行时读取 Core/Provider JSON Schema 与
JSON-RPC manifest，完成 `$ref` 解析、约束检查、fixture 校验、IR 构建和 Rust source
生成；不是把仓库中预生成的 `generated.rs` 复制到输出目录。

JSON Schema 描述 DTO 和约束；相邻 manifest 描述 method、request/response、event、
direction、capability、dispatch lane 和 transport。两者共同构成可以生成完整 JSON-RPC
SDK 的协议输入。

发布时使用 Bun 把入口、编译器模块、Rust package scaffold、stdio runtime 和本指南
编译进单体原生可执行文件：

```sh
bun build tools/cp-sdk-gen/cp-sdk-gen.mjs \
  --compile \
  --outfile ./cp-sdk-gen
```

Windows 输出名为 `cp-sdk-gen.exe`。Code Pet staging 会为目标平台执行 Bun compile；
macOS universal 构建分别产生 arm64 和 x86_64 executable，再通过 `lipo` 合并。最终
生成器运行时不需要 Bun、Node.js、网络或 Code Pet 源码仓库。

## 3. 导出 Rust SDK

从安装后的 macOS App 导出：

```sh
"/Applications/Code Pet.app/Contents/Resources/provider-sdk/cp-sdk-gen" \
  --package provider \
  --role server \
  --lang rust \
  --output ./codepet-sdk
```

安装包内的生成器默认读取同目录的 `protocol/`。也可以显式指定任意兼容协议包：

```sh
cp-sdk-gen \
  --protocol ./protocol \
  --package provider \
  --role server \
  --lang rust \
  --output ./codepet-sdk
```

在 Code Pet 源码仓库中开发时也可以运行：

```sh
npm run cp-sdk-gen -- --package provider --role server --lang rust --output ./codepet-sdk
```

输出是一个自包含 Cargo workspace：

```text
codepet-sdk/
├── Cargo.toml
├── README.md
├── cp-sdk-gen.lock.json
├── codepet-core-sdk/
└── codepet-provider-sdk/
```

不要修改生成目录中的 SDK 文件。升级 Code Pet 后，用新 App 中的生成器重新导出；CI
可以使用下面的命令检查目录是否仍与生成器匹配：

```sh
cp-sdk-gen --package provider --role server --lang rust --output ./codepet-sdk --check
cargo check --manifest-path ./codepet-sdk/Cargo.toml
```

当前 Provider server target 只支持 `--lang rust`。Dart 用于 Gateway/Remote client，
不是 Provider server target；未实现语言会明确失败。

## 4. 创建 Provider 工程

建议把 Provider 工程与生成目录并列，避免修改生成 SDK workspace：

```text
my-provider-workspace/
├── codepet-sdk/        # cp-sdk-gen 输出，不手改
└── my-provider/        # 你的独立 Provider 工程
```

`my-provider/Cargo.toml`：

```toml
[package]
name = "my-codepet-provider"
version = "0.1.0"
edition = "2021"

[dependencies]
codepet-provider-sdk = { path = "../codepet-sdk/codepet-provider-sdk" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

SDK 公开以下主要边界：

- `Provider` trait：所有 Host → Provider 方法的 typed interface。
- request/response/model 类型：从 JSON Schema 与 method manifest 生成。
- `ProviderEventSink`：发布 typed Provider → Host 事件。
- `serve_stdio`：JSON-RPC 2.0 JSON 序列化、Provider Frame V1、自动 raw/zstd、最终
  encoded-frame 限制、并发队列、控制通路、错误响应和 stdout 串行写入。
- `StdioServerOptions`：最终 frame 大小、普通/控制请求并发数和 drain timeout；Provider
  业务代码通常直接使用默认值。

最小入口结构如下：

```rust
use codepet_provider_sdk::{
    serve_stdio, Provider, ProviderDescribeRequest, ProviderDescribeResponse,
    ProviderEventSink, ProviderInitializeRequest, ProviderInitializeResponse,
    ProviderPluginDescriptor, ProtocolFuture, StdioServerOptions, VersionRange,
    PROTOCOL_VERSION,
};
use std::sync::Arc;

struct MyProvider {
    events: Arc<dyn ProviderEventSink>,
}

impl MyProvider {
    fn new(events: Arc<dyn ProviderEventSink>) -> Self {
        Self { events }
    }

    fn descriptor() -> ProviderPluginDescriptor {
        ProviderPluginDescriptor {
            plugin_id: "com.example.my-provider".to_string(),
            display_name: "My Provider".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            supported_versions: VersionRange {
                min_version: PROTOCOL_VERSION,
                max_version: PROTOCOL_VERSION,
            },
            instance_kinds: vec!["my-harness".to_string()],
        }
    }
}

impl Provider for MyProvider {
    fn provider_initialize<'a>(
        &'a self,
        request: ProviderInitializeRequest,
    ) -> ProtocolFuture<'a, ProviderInitializeResponse> {
        Box::pin(async move {
            if request.supported_versions.min_version > PROTOCOL_VERSION
                || request.supported_versions.max_version < PROTOCOL_VERSION
            {
                return Err(codepet_provider_sdk::ProtocolError {
                    code: "unsupported_protocol_version".to_string(),
                    message: "Provider protocol v1 is not supported by the Host".to_string(),
                    retryable: false,
                    details: None,
                });
            }
            Ok(ProviderInitializeResponse {
                selected_version: PROTOCOL_VERSION,
                plugin: Self::descriptor(),
            })
        })
    }

    fn provider_describe<'a>(
        &'a self,
        _request: ProviderDescribeRequest,
    ) -> ProtocolFuture<'a, ProviderDescribeResponse> {
        Box::pin(async { Ok(ProviderDescribeResponse { plugin: Self::descriptor() }) })
    }

    // 继续实现 instance.*、conversation.*、turn.*、approval.resolve 和
    // provider.shutdown。未覆盖的方法由 SDK 返回 method_not_implemented。
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(error) = serve_stdio(StdioServerOptions::default(), MyProvider::new).await {
        eprintln!("Provider stopped: {error}");
        std::process::exit(1);
    }
}
```

这段代码只完成协议握手，用于说明 SDK 边界。可用 Provider 至少还要实现实例创建、
启动、停止、销毁和 capability 查询；然后根据声明的 capability 实现对应的
`conversation.*`、`turn.*` 或 `approval.resolve` 方法。不能声明一个 capability 却让
对应方法保持 `method_not_implemented`。

Provider 业务代码不应读取 stdin 或写 stdout；stdout 完全属于 SDK 的 JSON-RPC
transport。诊断日志写 stderr。异步状态变化通过 `self.events.publish(ProtocolEvent::...)`
发送 typed event。

## 5. 编写插件 manifest

在 Provider binary 同目录创建 `codepet-provider.json`：

```json
{
  "manifestVersion": 1,
  "pluginId": "com.example.my-provider",
  "displayName": "My Provider",
  "executable": "my-codepet-provider",
  "args": [],
  "env": {},
  "enabled": true,
  "instances": [
    {
      "instanceId": "my-provider",
      "instanceKind": "my-harness",
      "displayName": "My Provider",
      "settings": {},
      "enabled": true
    }
  ]
}
```

约束：

- `manifestVersion` 当前必须为 `1`。
- manifest 的 `pluginId` 必须与 `provider.initialize/provider.describe` 返回值一致。
- `instanceKind` 必须出现在 descriptor 的 `instanceKinds` 中。
- `executable` 推荐使用 manifest 同目录的相对路径；Windows 写 `.exe`。
- Provider 专用配置放在 `instances[].settings`，由 `instance.create` 解码和校验。
- 同一发现范围内出现重复 `pluginId` 时 Host 会 fail closed，不会随机选择一个。

## 6. 独立进程测试

先直接构建并启动 Provider，确认 stdout 每行只有一个 JSON-RPC message：

```sh
cargo build --release --manifest-path ./my-provider/Cargo.toml
./my-provider/target/release/my-codepet-provider
```

测试程序应启动子进程、保留 stdin/stdout pipe，并依次覆盖：

1. `provider.initialize` 和 `provider.describe`；
2. `instance.create`、`instance.start`、`instance.capabilities`；
3. Provider 声明支持的 conversation/turn/approval 方法；
4. typed event 是否从 stdout 返回；
5. `instance.stop`、`instance.destroy`、`provider.shutdown`；
6. 关闭 stdin 后进程能否退出，stderr 中是否有异常。

可直接使用 `protocol/provider/v1/fixtures` 作为合法 wire message 的起点。JSON-RPC
请求必须使用 `"jsonrpc":"2.0"`、唯一 `id`、manifest 中的方法名以及匹配 schema 的
`params`，每条消息以单个换行结束。

## 7. 安装并让 Code Pet 发现

构建完成后准备一个只包含 binary 与 manifest 的目录：

```text
my-provider/
├── codepet-provider.json
└── my-codepet-provider       # Windows 为 my-codepet-provider.exe
```

有两种接入方式：

1. 复制到当前 Code Pet 应用数据目录下的
   `provider-plugins/my-provider/`；应用数据目录可以在 Code Pet 设置中修改。
2. 把 `my-provider/` 或它的父目录加入
   `settings.providerPlugins.directories`。相对路径按当前应用数据目录解析。

重启 Code Pet 后，Host 会扫描根 manifest、一级子目录中的
`codepet-provider.json`，以及 `*.codepet-provider.json`。发现后按 manifest 启动进程，
执行协议握手并创建声明为 enabled 的实例。接入失败时优先检查：

- binary 是否存在且在当前平台可执行；
- manifest 是否包含未知字段、无效 `pluginId` 或重复 `pluginId`；
- stdout 是否混入日志或非 JSON 内容；
- descriptor、manifest 和 instance settings 是否一致；
- `cp-sdk-gen.lock.json` 是否来自当前 Code Pet 版本。

## 8. 协议演进

Provider 必须以 `provider.initialize` 协商版本，不应根据 Code Pet 版本字符串猜协议。
升级 SDK 时重新运行生成器并提交新的 `cp-sdk-gen.lock.json`。新增字段、方法或事件以
App Resources 中的 schema/manifest 为准；SDK 生成文件和 transport runtime 不应在
Provider 项目里复制维护。
