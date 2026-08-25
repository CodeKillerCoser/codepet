# Code Pet Standard Protocol v0

## 背景

桌宠与未来远程客户端需要共享同一套 Runtime Gateway 契约。v0 只建立可生成、可编译、可检查漂移的协议边界，不接入任何 Provider runtime 或 Transport 业务实现。

## 事实来源与版本

`schemas/v0.json` 是 DTO 的唯一事实来源，使用 JSON Schema Draft 2020-12。`manifest.json` 只描述协议版本、RPC 方法的 request/response 配对及 server event payload，不重复声明字段。Rust 与 TypeScript 文件都是机械生成产物，带有禁止手改标记。

wire `protocolVersion` 当前为 `0`，它标识一份确定的生成契约。改变 DTO、方法集合或事件集合时必须提升协议版本并同步更新 fixture；Provider-advertised string 和受控 extension data 可以在同一版本内扩展。握手由 `protocol.handshake` 协商客户端支持范围，事件用单调 `eventSequence` 标识顺序。

## v0 范围

- Provider：identity、status、capability、模型与 reasoning effort 的 Provider-advertised string，以及显式的 JSON extension point。
- Conversation：摘要、运行状态、权限级别、当前 TurnTask，以及可选 `workspaceRoot`。省略 `workspaceRoot` 表示普通聊天，有值表示以该目录作为 Provider 会话上下文的项目会话。
- TurnTask：一次可运行任务的 identity、状态和可展示摘要；`turn.send` 同时承载普通回复、快捷回复 identity 和 steer target。
- Approval：待审批内容、允许的 `approve`/`deny` 决定和解决状态。
- 基础 ProtocolError、分页 cursor 和 event sequence。

权限只有 `read-only`、`workspace-write`、`full-access`。v0 不包含 usage、Diff、文件浏览、终端、远程加密、Dart SDK 或 Provider runtime。

`conversation.create` 原样携带可选 `workspaceRoot`，供后续 Provider adapter 映射到 Codex `thread/start` 的 `cwd`；协议不因此引入 Project 实体，也不授予目录浏览能力。

## 生成边界

- Rust：`src-tauri/src/runtime_gateway/generated.rs`，包含 serde DTO、tagged request/response/event、`ProtocolServer` 与 dispatcher 壳。
- TypeScript：`frontend/lib/generated/runtimeGateway.ts`，包含 DTO、tagged union、typed client、transport contract 与 event map。
- 共享样例：`protocol/fixtures/`，由生成检查校验，并由 Rust 测试执行 serde round-trip。

## 命令

```sh
npm run protocol:generate
npm run protocol:check
```

`protocol:generate` 在校验 manifest、所有 ref、方法/事件名称唯一性、受支持 schema 子集和 fixture 后重写生成文件。`protocol:check` 执行相同校验，但不写文件；生成内容与仓库不一致时返回失败。

## 风险与验证

- Rust/TypeScript 漂移：运行 `npm run protocol:check`。
- schema 或 ref 失效：生成器在输出前拒绝未解析 ref、非法 Draft 标记和不受支持关键字。
- wire fixture 与 serde 行为不一致：运行 `cargo test --manifest-path src-tauri/Cargo.toml --test runtime_gateway_protocol_tests`。
- 前端生成类型不可导入：运行 `npx tsc --noEmit --incremental false --target ES2022 --module ESNext --moduleResolution Bundler frontend/lib/generated/runtimeGateway.ts` 和 `npm run build`。

## 未知项

Transport framing、Provider 原生 payload 映射和事件重放存储由后续阶段确定；它们不得绕过或复制这里的 wire DTO。
