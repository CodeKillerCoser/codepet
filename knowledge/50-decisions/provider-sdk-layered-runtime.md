# Provider SDK 按传输、消息适配与服务运行时分层

## 背景

2026-09-06 引入 STDIO mux 后，原 `mux.rs` 同时承担 bootstrap、Yamux driver、物理帧保护、容量管理、JSON/zstd 编解码、控制方法分类和 RPC 超限响应。原 `stdio.rs` 定义两个运行时共享的配置、错误和事件接口；它选择 mux，而 mux 又依赖其中的配置校验。继续增加语言 SDK 或维护取消、退出逻辑时，这种职责交叉会扩大修改范围。

相关协议与生命周期约束见[消息复用设计](../10-architecture/provider-stdio-message-multiplexing.md)。分层重构后，用户进一步明确仅保留 mux：旧 runtime 和 Host 整帧分支已删除；业务 JSON-RPC、mux 握手字节和容量默认值不变。

## 决策

保留一个 `codepet-provider-sdk` crate，通过模块依赖表达以下职责：

| 模块 | 职责与依赖 |
| --- | --- |
| `generated.rs` | canonical schema/manifest 生成的 DTO、业务 trait、dispatcher 和 JSON codec；继续由生成器维护 |
| `transport/` | 传输合同、CPRF 帧头、bootstrap、Yamux 连接与 stream、字节/并发/worker 额度和 I/O 超时；生产代码不引用 Provider 业务消息或服务运行时 |
| `message/` | Host 与 Provider 共用的消息适配：实现传输层定义的 `MessageCodec`，判断控制方法，执行 Provider JSON/raw/zstd 策略，将错误映射为业务协议响应 |
| `runtime/` | 公共配置、事件入口、mux STDIO 接入、阻塞 I/O bridge、心跳、业务派发与退出；只保留 mux 服务循环 |
| `content/` | Provider mapper 可选的内容处理策略；传输和服务运行时不调用此层裁剪业务对象 |
| `lib.rs` | 稳定公开入口，显式重导出手写 API；现有调用方无需跟随内部目录移动 |

主要调用方向为 `runtime → message → transport`。运行时可直接使用传输配置，所有需要协议数据的模块可依赖生成合同。传输层读取的 `ProviderTransportHello/Selection/Limits` 是传输 DTO；这不授权其依赖 `ProtocolMethod`、`ProviderWireMessage` 或业务 handler。

### 依赖倒置与资源所有权

`transport::mux::MessageCodec` 定义关联消息类型、错误类型、编码/解码以及通道分类接口。`message::codec::ProviderMessageCodec` 实现它，传输核心仅调用该接口。`TransportError` 不包含 RPC id、业务错误码或响应构造策略，上层转换为现有 `ProtocolError`。

编码、解码仍由传输层在取得 worker 和字节额度后调用；不能因为拆层而让上层先构造不计费的大型临时缓冲。`Received<M>` 持有接收额度，消息所有者释放时归还；后台编码/解码即使被取消等待，也继续持有额度直至 worker 结束。Provider 层只在消息编码失败、尚未写 envelope 时选择超限响应。

`ProviderMux`、`MuxIncoming`、`MuxMessage`、`MuxDriver` 是具体 Provider 适配的类型别名，维持原方法签名和公开入口。通用实现仍是 crate 内部模块，没有新增供第三方选择任意 codec 的协议入口。测试用独立字节 codec 验证机制解耦，不代表生产 profile 接受非 JSON 内容。

### 生成与分发

`cp-sdk-gen/provider-runtime.mjs` 集中登记需要 Bun 静态嵌入的手写源文件及相对路径。生成器按该清单写出嵌套模块，不在运行时扫描开发仓库。导出测试递归读取实际 Rust `src/`，排除 `generated.rs` 后检查清单完整性，并逐文件比较导出内容；独立 `generated.rs` 仍从 canonical 输入编译产生。

## 备选方案

- 只移动文件：成本较低，但无法消除传输核心对 Provider 业务策略的依赖。
- 立即拆成多个 crate 或提供 Python/Node 原生绑定：可以形成独立分发单元，但当前没有对应运行时与发布需求；会增加公开 API、版本、构建和平台包维护成本。
- 保留双运行时：有利于旧二进制兼容，但用户已决定统一使用 mux；删除旧服务循环，避免重复维护生命周期和测试。

## 取舍理由

先在单 crate 内实现依赖倒置，既保留 Host 和 Provider 的现有 API，也让将来的传输核心抽取有明确接缝。共享 CPRF 帧头机制，分别保留旧消息 codec 与 mux 有界编码策略；相同字节格式不意味着相同容量政策。生成大文件的维护权威是 schema/codegen，手工拆分它的收益较低。

## 影响范围与验证

- Provider SDK：模块路径、可见性和类型别名可能影响公开调用；运行 SDK 测试、Host 全量测试及三个内置 Provider 端到端测试。
- 传输核心与消息适配：拆层可能提前释放额度或丢失控制通道校验；保留原阻塞大流、取消、失活、解压和 shutdown 回归，新增独立字节 codec 的额度生命周期测试，以及 Provider codec 错误占用控制通道的拒绝测试。
- SDK 生成器：嵌套源码可能遗漏；检查完整导出清单，编译独立生成器，从仓库外执行导出和 freshness 检查，再独立编译及运行导出包的本地单元测试。
- Host 与三个 Provider：Host 只连接 mux，Provider 入口维持公共 API；独立二进制测试改用共享 mux 客户端，并验证初始化、心跳、请求/事件和清理。
- 架构文档与复用规约：更新入口路径，避免后续维护者继续把策略放回传输层。

以下为删除旧运行时前的重构验证记录；当前 mux-only 结果见 [验证 runbook](../40-runbooks/provider-stdio-mux-validation.md)。2026-09-06 已运行：重构前 SDK 38 项测试；重构后 SDK 40 项测试；Host 69 项测试（含三个内置 Provider 的同一端到端用例）；protocol freshness 与 17 项测试；SDK 导出 3 项测试。均通过。Host 构建仍有既存的 Codex `CodexThreadItem` unused-import 警告，SDK 本次构建无新增警告。

执行命令（仓库根目录）：

```sh
cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-provider-sdk
cargo test --manifest-path crates/Cargo.toml -p codepet-host
npm run protocol:check
bun test tools/cp-sdk-gen/cp-sdk-gen.test.mjs
bun build tools/cp-sdk-gen/cp-sdk-gen.mjs --compile --outfile /tmp/codepet-sdk-refactor-20260906/cp-sdk-gen
git diff --check
```

另外，将 canonical `protocol/` 复制至 `/tmp/codepet-sdk-refactor-20260906/protocol` 后，在该临时目录运行以下命令。编译后的生成器成功导出、freshness 检查通过，导出 workspace 离线独立编译成功，Provider 包 16 项源码内单元测试全部通过：

```sh
./cp-sdk-gen --package provider --role server --lang rust --protocol ./protocol --output ./exported-sdk
./cp-sdk-gen --package provider --role server --lang rust --protocol ./protocol --output ./exported-sdk --check
cargo check --offline --manifest-path exported-sdk/Cargo.toml --workspace
cargo test --offline --manifest-path exported-sdk/Cargo.toml -p codepet-provider-sdk --lib
```

## 后续观察

当前实现仍使用 Rust、Tokio、Yamux 与 CodePet 传输 DTO；模块解耦不等于已完成独立 crate、FFI 或 Python/TS SDK。若出现第二个生产 codec、另一种底层 I/O 或原生语言绑定需求，再依据实际使用验证并抽取公共包。平台验证仍限于本次 macOS 环境，不据此声称 Windows/Linux 原生 pipe 已完成验证。

## Provider 业务持久化补充

用量与未读 SQL 实现在 `crates/providers/codepet-provider-data`，不进入 SDK runtime。Host 为每个插件注入独立数据库路径，业务表由该库维护；参见 [Provider 用量查询与业务存储](../10-architecture/provider-usage-query-and-storage.md)。
