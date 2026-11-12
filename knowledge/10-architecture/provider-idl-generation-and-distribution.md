# Provider IDL 统一生成与分发

> 后续整理：权威输入已迁至 `protocol/v1/`，Provider/Gateway 按功能分组，共享对象中立维护。下文保留前一轮迁入与验收记录；最新组织见 [模块化协议 IDL](modular-provider-and-gateway-idl.md)。对外 SDK 布局保持 provider-sdk/v1。

状态：2026-11-12，已迁入仓库；验收记录见下文。本轮沿用已确认的 SDK 版本目录、模块划分及精简 README，不改变现有业务报文、profile ID 或内置 Provider 的进程边界。

## 背景与目标

审阅阶段的 SDK 只在 Downloads 中重排；仓库的 DTO、方法注册表、发布脚本与传输实现仍有分别维护的定义。尤其 envelope 在导出脚本中拼装，安装清单另有手写 Host 结构，mux 限制和心跳周期散落在 Rust 中。目标是让确认后的 IDL 成为唯一事实来源，代码和发布包都从它确定性地产出，禁止以兼容文件或下载副本反向定义协议。

## 范围

迁入 `protocol/provider-sdk/v1/`：SDK 发布清单、README、`protocol/manifest.schema.json` 和 `protocol/sdk/`。sdk 中按 common、agent-domain、transport、rpc、provider 分工，目录版本只在 SDK 层出现。安装清单由插件作者填写，不向外生成插件业务代码；Host 内部可从清单 Schema 生成解析结构与校验器。

不实现新的 Observation 业务，不调整 Gateway/手机网络通信，不为独立作者建设新的通用 Provider 反向运行时，不自动安装 Node/pi 等依赖。已有 v1 安装清单和 Rust 公共路径保持兼容。

## 证据与实现路径

`tools/protocol-codegen/provider-contract.mjs` 统一加载权威 IDL，检查 description、引用闭包、协议默认值、预算约束和可支持的 envelope 格式。`protocol:generate` 的模型读取使用此输入；原 core/agent/provider 三份 Schema 和 Provider service manifest 仅是生成的兼容视图，文件中标明来源。旧目录中的手写 profile、插件 Schema 和作者说明已移除。没有权威入口时，仓库默认生成不能静默退回兼容视图。

methods/events 注册表是方法及事件清单的唯一来源。RPC envelope 的权威输入只定义通用外壳，发布时从注册表派生按方法区分的请求/事件联合；不在两份 IDL 同时维护方法名。插件清单的 transport 枚举从 profile 注册表派生。methods/events 的发布 version 来自 SDK 清单。内部 service 元数据保存在权威 methods 文件中，兼容 manifest 不另手写方法表。

传输 JSON IDL 定义 magic、版本、阶段、控制字节、字段顺序和宽度、必选 features 与压缩策略；握手 Schema 定义限制范围、默认值和跨字段预算约束。`provider-runtime.mjs` 生成传输常量、偏移、默认结构和限制校验，Rust framing、mux、压缩与握手机制消费它们。生命周期 Schema 的策略注解生成心跳常量；README 引用同一输入渲染周期、握手限制和 features，避免再维护一份协议值。

安装 Schema 编译为 Host 私有清单结构和 v2 校验器，保留现有 Rust `executable` 字段及 command 别名作为内部 API 兼容。校验编译器遇到未支持的关键字直接失败，不能悄悄忽略约束。包路径、URL、进程所有权等业务语义仍由 Host 机制实现，不把所有机制代码都伪装成 Schema 生成结果。

`export_provider_sdk.mjs` 不再定义 envelope、方法或 DTO，只调用同一个模型和发布器，输出 `provider-sdk/v1/`。staging 使用同一出口；只分发公开合同及 README。SDK 清单携带源摘要，供产物、独立示例和 App 资源校验一致性。对外包提供 IDL 输入，语言代码生成工具仍由作者选择；仓库内部 cp-sdk-gen 是独立开发工具。

## 涉及模块与风险

| 模块 | 原因、风险及验证路径 |
| --- | --- |
| 权威 IDL、统一加载器、生成器 | 核心是消除第二份事实；临时修改心跳、限制默认值和方法名，断言运行时代码、兼容视图、发布 envelope 和 README 一起变化 |
| 核心/Agent/Provider 生成 DTO 与旧视图 | Gateway、桌面和已有 Rust 调用方仍引用旧模块；执行原协议 fixtures、SDK lib 与 Host 回归，检查新鲜度 |
| 独立 transport 与 Provider heartbeat | 生成的值取代原常量，可能破坏 wire；执行分帧、有界解压、Yamux 交互与并发隔离测试，以及三个内置 Provider 集成 |
| Host catalog 与导入 | 清单结构和校验生成可能影响 v1、command/args/env；执行 catalog、manager、真实自定义包导入及 CLI 验收 |
| 导出/staging/CI | 新版本目录可能遗漏文件或混入旧资源；检查白名单、相对 ref、SDK 清单入口与 App bundle 中每个文件内容 |
| 独立作者示例 | 不能用 Host 源码补足公开合同；子 agent 只读导出的 SDK 和自己的源码，使用 sdk-manifest 发现类型/注册表后重新打包，再由父 agent 从 Host 黑盒验证 |

## 验收记录

本机复用 `CARGO_TARGET_DIR=/Users/wangxin/.codex/worktrees/355e/codepet/crates/target`，仅作为编译缓存，不读取该工作区的源码。真实内置 OpenCode fixture 的 HTTP 使用 `NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost`，避免机器既有代理影响回环请求。

- `npm run protocol:check`：原协议 23 项及新增权威 IDL 4 项，共 27 项通过；覆盖错误引用、缺失说明、非法默认值、envelope 形状、安装 Schema 未支持关键字拒绝及产出确定性。
- `npm run providers:test`：5 项通过，验证新目录、分发边界与引用闭合。Bun sdkgen 测试 3 项通过。
- 独立 transport 12 项、Rust Provider SDK 41 项通过；SDK 中 5 个既有可选测试忽略。仓库外导出的 Rust workspace 同样通过 41+12 项。
- Host lib 107 项、manager_gateway 46 项、process_rpc 8 项、provider_profiles 2 项、三个内置 Provider 集成 4 项通过。
- 显式 opt-in 的独立 pi/DeepSeek Flash 测试 1 项通过：生产 import/Gateway、真实 pi RPC/read 工具、模型确认、delta 与完整历史，无 Rust bridge，不打印密钥。
- 独立子 agent 使用最终公开 SDK，源码和四个 bundle 各 15 项通过。只按 SDK 清单与相对引用发现合同，插件 ID、profile 及 selectionRequired:false 保持。父 agent 按安全归档逐项检查后原样复制交付物，不代写插件实现。
- 原生 macOS ARM64 App 构建及本地签名检查通过。bundle 中 SDK 文件集合和内容与统一发布器逐一比较：12 个文件，根为 provider-sdk/v1，无旧布局或私有物料。

最终源摘要为 `sha256:2ba20be7f4a19b375e277fc04a874df06ee707d707104e732908bb744097f8e1`。独立插件最终 SDK 快照与发布结果相同，四个 bundle 的 SHA256 为 `84a713e48065c0c36d70bfe3a3d2d45704425ee5540a916f1bfbb6fffa02da06`。最终广域检查 `cargo check --manifest-path crates/Cargo.toml --workspace --all-targets` 通过；最终一轮 Host lib/集成和协议 freshness 再次通过，`git diff --check` 通过。

## 知识沉淀与未知项

长期约束见 `knowledge/60-rules/provider-idl-single-source.md`；后续新增字段、方法、profile 或策略参数先改权威 IDL，再生成和验证。旧分发评估与审阅包是历史证据，不能作为协议输入。

本轮没有替换已安装 App，没有在 Windows/Linux 做原生实机验收，也未声明新的生产性能指标。JSON Schema 约束之外的进程管理、业务状态机、超时取消和流控仍有手写实现，需要互通测试约束；唯一事实来源不等于可以只生成 DTO 就完成 Provider。
