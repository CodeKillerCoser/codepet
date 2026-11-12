# Provider 三种传输 Profile 与独立合同 SDK

> 2026-11-12 后续：确认后的 IDL 已迁入 `protocol/provider-sdk/v1/`，统一生成、兼容视图和新版分发验收见 [Provider IDL 统一生成与分发](provider-idl-generation-and-distribution.md)。本文保留早期提取与初次验收事实，旧目录不是当前协议源。

状态：2026-11-11，已实现，最终验收命令和安装结果见本文末尾。用户已将早期 mux-only 决策调整为三个显式 profile；本轮按这个方向收敛 Host，Provider 业务运行时的通用反向 pipeline 暂不建设。

## 背景

已有 mux 分层可复用，但 Host 进程适配器只接受 mux，简单通知插件也被迫实现 Yamux。随 App 分发的 SDK 还携带 Gateway/LAN、测试 fixtures 和 Rust 生成器。上轮 TS pi 示例用 Rust bridge 绕过传输，不能证明独立作者只看合同就能开发。Host 事件又经共享队列同步处理，存在跨 Provider 等待点。

## 目标

三种 stdio profile 共用消息与业务合同；收包、定界、解压、反序列化可按 profile 装配并单独测试。每个 Provider 有独立连接、预算、请求表、超时和事件处理，不把业务处理串到全局循环。对外只提供完整 JSON Schema、方法/事件表和文档，以不接触内部源码的子 agent 实现验证。

## 非目标

不改远程 Gateway/RTC/LAN，不建设网络插件协议，不迁移任务谱系或实现完整 Observation 能力，不为新 profile 增加内置 Provider 的反向服务运行时。保留三个内置 Provider 的 mux wire、业务协议版本和既有 Rust SDK 公共入口；独立开发者可按合同自己实现相应发送端。

## 现状理解与模块边界

| 模块 | 为什么相关与最终责任 |
| --- | --- |
| `sdk/rust/codepet-transport` | 新独立 crate，承接原 transport 机制；只依赖 common DTO、I/O、Yamux、zstd，不依赖 Host、Provider 方法或处理器。包含 profile、JSONL/CPRF 定界、payload 编码、容量和取消/关闭所有权 |
| `sdk/rust/codepet-core-sdk`、`protocol/core/v1` | 三个 bootstrap DTO 从 Provider schema 移入 common；字段及线上 JSON 不变，Rust Provider 保留 re-export |
| `sdk/rust/codepet-provider-sdk/message` | 向独立传输注入 Provider JSON codec、控制方法分类和超限响应策略。原 transport 路径仅兼容 facade，原 mux runtime 仍用于内置 Provider |
| `codepet-host/providers/process/transport` | Host profile 装配与 RPC 关联；三 profile 统一为连接 exchange/close 和入站事件接口；Serial 请求 ID 表独立于读循环 |
| `providers/manager`、`gateway/provider_updates` | 每 Provider 接收和有序业务 worker；共享 inbox 只分流，重处理在独立阻塞任务执行；批量启动/退出并行，保留每插件操作锁 |
| `scripts/export_provider_sdk.mjs`、staging | 发布包只含 Provider 相关合同，不带 Gateway、LAN、fixture、Rust/TS 源码或 cp-sdk-gen 二进制；开发用 Rust 生成器仍可独立导出并编译 |

## 实现路径

JSONL：stdio → LF 定界 → 注入的 JSON codec。zstd：stdio → CPRF 定界 → 有界 zstd 解压 → 同一个 JSON codec。mux：stdio → CPMX bootstrap → Yamux 流 → envelope/额度准入 → CPRF → raw/zstd payload → JSON codec。发送在 Host 使用相反顺序的必要组成；没有给外部作者新增通用 Provider server pipeline。

JSONL 最大 16 MiB（含 LF）；zstd/mux 编码消息上限 16 MiB、解码 JSON 上限 128 MiB。普通 zstd 每消息独立 level 1；mux sender 按大小选择低/较高等级，并在节省不足时保留 raw，接收端仅依赖 raw/zstd 格式与容量。所有 profile 均先按元数据/固定上限约束分配与解压，部分包失活与 EOF 有明确错误。

发送对象进入 Serial 队列前取得额度，取消后队列/worker 继续持有自己的额度。接收解码 worker 持有额度直到完成；`ProviderInboundMessage` 和内部 HostUpdate 把接收 guard 传到事件业务处理结束，避免将大对象转移到不计费的业务队列。配额计算针对消息字节，不是整个进程 RSS。

每个 Provider 的读写和重处理独立。事件 worker 内保持顺序，跨 Provider 并发；积压达到限制只处理对应连接。Gateway 全局游标分配、轻量 Arc 留存和广播次序在同一短临界区完成；大对象复制/销毁放到锁外，避免并发发布时游标乱序或丢掉较小游标。共享数据库短事务仍按数据库规则执行，不能在事务中包入传输等待或解压工作。

## Manifest 与对外 SDK

v2 明确 `command`、`args`、`env`、`transport`，插件身份和实例配置沿用原字段。无隐式 shell 展开，argv/env 保持字面值。工作目录统一为安装包根目录；包内命令随包迁移，v2 外部绝对程序/PATH 程序作为运行环境依赖，不被复制。旧 v1/executable 仍可用，缺 transport 默认 mux；新 v2 必须显式声明。Host 保留自身上下文环境变量，拒绝冲突声明。

导出包含：三个业务/common schema、`plugin-manifest.schema.json`、`profiles.json`、从注册表生成的 `wire.schema.json` 和 methods/events、索引与作者说明。响应 result 使用关联请求的方法 response schema 校验。构建 staging 使用同一导出函数，并清理旧 SDK 物料；独立 CLI 要求空目标目录，防止混入私有文件。

新示例 `examples/providers/independent` 来自 fork 无历史上下文的子 agent。只给它 `/tmp/codepet-provider-contract-sdk` 的导出文件和业务要求，明确禁止读取仓库源码/knowledge/旧示例；公开依赖及 pi RPC 文档允许使用。它独立实现三个 profile，bundle 单文件无需 node_modules，并另做 pi JSONL 包。父 agent 仅编写 Host 测试和配置 manifest，未代写插件实现。

这次互通发现插件把 small 请求 class 继承给大响应。子 agent 按公开合同修复，增加 128 KiB 历史回归；文档明确只有 control 回复保留 lane，其余按响应自身大小分类。这是 SDK 外部作者验收的实际反馈。

## 风险与验证路径

- 提取破坏内置 mux：执行 SDK/Host 回归、三个内置 Provider 集成，以及导出 Rust workspace 独立编译测试。
- 拆包/EOF/解压错误：独立 crate 覆盖分片合包、UTF-8/U+2028、超长头部、截断、错误编码、解压上限；Host 保证最终响应先于 EOF 被消费。
- 跨 Provider 阻塞：阻塞 A 解码、填满 A 业务队列、延迟 A 请求或让 A 输出半包，验证 B 继续；批量启动加慢 Initialize 回归。
- 全局事件乱序：8 个并发发布者产生 800 个事件，校验 live/replay 游标完整有序。
- SDK 偷带私有物料：检查文件白名单、所有本地 $ref 闭合、profile 枚举一致、envelope 数量来自 methods/events；拒绝非空导出目录。
- 外部运行时依赖：Node24+验证插件由 manifest 明确依赖，Host 自身不需要 Node；自包含其他语言程序同样按进程合同接入。Windows/Linux 原生 App 另行实机验收。

## 测试计划与验收记录

独立传输 crate、Rust SDK lib、Host lib/manager/process/profile 测试、协议 freshness、staging/export 和独立作者测试均纳入验收。新 CI 使用 Node24，执行独立模块与三种外部 profile 互通；该工作流尚未在远端运行。真实模型测试显式 opt-in，默认测试不消费模型。

本机真实 pi 测试已通过：JSONL/v2 包经生产 import/Gateway 调用独立 pi RPC，模型字段确认为 DeepSeek Flash，实际读取生成的 proof.txt 并返回精确内容，随后能读取完整历史；没有 Rust bridge，也没有读取/输出密钥。pi CLI 为显式外部依赖，认证由用户已有 pi 配置和 DS_API_KEY 解析命令负责。

### 2026-11-11 验证结果

以下均在本工作区运行。Cargo 为复用本机缓存指定 `CARGO_TARGET_DIR=/Users/wangxin/.codex/worktrees/355e/codepet/crates/target`；这只是产物目录，不读取另一工作区源码。

| 命令/路径 | 实际结果 |
| --- | --- |
| `cargo test --manifest-path sdk/rust/Cargo.toml -p codepet-transport -p codepet-provider-sdk --lib` | 独立 transport 12 通过；Provider SDK 41 通过、5 个既有测试忽略 |
| `cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib -- --test-threads=1` | 106 通过，包含队列隔离、并发游标与 manifest v2 |
| Host `--test manager_gateway --test process_rpc --test provider_profiles` | 分别 46、8、2 通过；profile 中真实模型测试默认忽略 |
| Host `--test builtin_provider_integration` | 4 通过，覆盖 Codex/Claude/OpenCode 原 mux 及释放/重启；本机设置 `NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost` 防止已有全局代理接管 fixture 回环 HTTP |
| `cargo check --manifest-path crates/Cargo.toml --workspace --all-targets` | 通过，包含原有测试、示例的公开 re-export 检查 |
| `node tools/protocol-codegen/generate.mjs --check` 与 protocol/staging/export Node tests | freshness 通过；23+3+2 项通过 |
| `bun test tools/cp-sdk-gen/cp-sdk-gen.test.mjs`、仓库外 Rust SDK 导出 workspace lib tests | 3 项生成器通过；导出 workspace 的 41+12 项通过，无 Host 依赖 |
| `PROVIDER_TEST_DIRECTORY=<独立包 jsonl> npm test --prefix examples/providers/independent` | 11 项通过，包含公共 Yamux client 与 bundled 包 |
| Host `schema_only_pi_plugin_runs_deepseek_flash_through_jsonrpc_profile -- --ignored --nocapture` | 显式运行 1 项通过；`CODEPET_TEST_PI_COMMAND` 指向独立安装的 pi CLI，实际模型 `deepseek-flash`；完整命令见测试文件的环境入口 |
| `npm run tauri -- build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'` | 前端、三个内置 Provider、Rust Host 和 macOS ARM64 App 构建通过；本地签名校验通过 |

原生验收通过实际文件选择器导入 `independent.echo.jsonl`、`independent.echo.zstd`、`independent.echo.mux`、`independent.pi.jsonl`，四个卡片均显示 Provider 在线、Harness 可用和导入成功。已安装新版 `/Applications/Code Pet.app`，旧版保留于 `/tmp/codepet-install-transport-backup.app`；安装二进制与本轮产物 SHA256 均为 `6ed2de210e0ba1ab2b5a4d64226a369689fb275ea69aa66559be4913132fa8ae`。

安装后 SDK 只有 10 个合同文件，包含 wire schema，不含 Gateway、channel、fixture 或生成器。安装后的四个 `provider.mjs` 与子 agent 交付的包逐一校验 SHA256 相同。导入配置为本机 Node 绝对路径；pi 额外指向独立安装的 CLI，并配置不含密钥的 PATH。旧自定义 Pi bridge 未删除，新包与它使用不同身份。

后续进展：运行时选择职责现已增加显式 `selectionRequired:false`，新版 App 原生验收确认误导提示消失；当前产物与回归记录见 [运行时职责修复](../40-runbooks/provider-self-managed-runtime.md)。以下保留首次验收发现的问题及示例能力边界。

首次验收发现两项非传输边界：独立 Echo/Pi 示例返回空 `runtime.getInstalled` 清单，当前运行时卡片仍显示“未找到兼容运行时”的提示，尽管实例 Ready、请求/输出与真实模型验证成功；需要后续按 Provider 是否依赖外部 Harness 收敛展示/检测语义。独立 pi 示例只映射文本、取消和内存历史，不承诺工具审批、附件或跨重启持久会话。两项均不伪装为已覆盖的能力。

默认自动测试不访问付费模型。Windows/Linux 原生 UI、远端 CI 执行、生产负载指标本轮没有验证。

## 知识沉淀

本文件替代早期 mux-only 约束，外部作者入口在 `protocol/provider/docs/README.md`。独立源码与测试保留，后续协议或 SDK 改动必须继续通过黑盒互通。旧 bridge 仅为历史实验，不作为新 SDK 示例。

## 未知项

尚未声明生产负载 p99/RSS 指标、Windows/Linux 原生 UI 验收或完整 Observation-only 业务合同已完成。本轮保留共享业务存储设计，不以传输并发承诺消除同一数据库的事务互斥。
