# Provider 与 Gateway 的前向兼容及版本协商

## 背景

用户于 2026-11-14 确认采用 version/minVersion，并要求先更新协议文档、IDL 和生成改造说明。现有 v1 用 supportedVersions 的 minVersion/maxVersion；Rust/Dart 生成 codec 对未知字段、枚举和联合分支严格拒绝。直接把现有握手改名而不适配所有端，会使旧端在版本检查之前解码失败。源码摘要只能追踪产物，不证明兼容。

## 决策

Provider 与 Gateway 独立声明连续区间 `[minVersion, version]`，两者均为正整数。响应方回报本端区间及共同最高 selectedVersion，请求方按同一公式验证：`min(local.version, peer.version) >= max(local.minVersion, peer.minVersion)`。不存在交集时不建立业务连接。协议范围在 protocol 对象内，软件版本维持 hostVersion/clientVersion/插件 version，避免字段冲突。

新合同位于 `protocol/v2`，明确 draft-not-generated；当前只是握手与兼容政策草案，没有完整服务和依赖闭包。旧 v1 仍是生成和运行源。本次不修改现有 DTO、运行时、传输版本和已发布 SDK。首次新握手以 2/2 为基线，因为旧严格 v1 无法直接理解它；如需并存，后续显式增加 v1 adapter，不能猜测回退或重试写请求。v2 中的 v1 类型引用只为复用未变字段，不代表已有 codec 兼容。

纯增量扩展增加 version、保持 minVersion；实现已有能力只更新能力声明，选项目录更新 capabilityRevision。破坏结构或既有语义时默认提高 minVersion，只有保留旧行为并通过兼容测试才可维持下界。新增枚举/联合若不能安全降级，需要能力门控或提高下界。

## 备选与取舍

继续严格拒绝所有未知内容会让每次字段扩展都成为破坏变更；仅交换 minVersion 又缺少最高支持信息。选择 version/minVersion 是为了减少重复上界字段，同时明确连续兼容承诺。没有新增通用自动降级、自动重放机制。

## 影响范围

- `protocol/v2/common`：范围、选择、跨字段关系；JSON Schema 不会自动执行两端跨对象比较，需生成/运行时校验。
- `protocol/v2/provider/lifecycle` 与 `gateway/session`：统一新握手字段和插件 descriptor 的一致性规则。
- 生成器加载器/IR、Rust/TS/Dart codec：未知字段接收忽略，未知枚举/联合保留原值，能力和事件有独立未知分类，已知非法数据继续拒绝。
- Host、Provider、操作端：选定版本后保持会话不变，未知状态不能变成空闲/授权/可执行；Gateway 合法事件 cursor 在跳过未知事件时继续推进。
- 发布与 CI：v2 不能覆盖 v1 产物；目录格式、服务业务版本和 SDK 格式要解耦。公开 Provider SDK 仍排除 Gateway；旧插件、手机与新端的组合行为须有 fixtures。

## 验证与后续观察

风险对应验证：新握手不能被旧端解析→首次版本与入口迁移测试；未知联合导致业务误执行→保守 fallback 测试；开放解码绕过预算→字节/深度/数量边界；已知非法数据被吞掉→区分 unknown 与 invalid；未知事件反复重放→cursor 前进；写请求重复执行→无自动重试；两层升级耦合→服务独立版本生成测试。

本轮只增加草案结构、引用闭包/description/旧入口隔离/协商示例检查，既有生成器行为不变。后续实施清单在 `protocol/v2/README.md`；开启 v2 前必须同时完成 schema、生成器、运行时、两端适配与发布验证，不能先宣称支持。

本轮验证结果：`npm run protocol:check` 35 项通过（含 3 项新草案检查），`npm run providers:test` 5 项通过，`git diff --check` 通过。未生成 v2 语言代码、未升级运行中的 App、未提交推送。

## 落地记录（2026-11-14）

当前 `protocol/v2/` 是完整活动源，Provider/Gateway 初始业务区间均为 2/2。`codegen.json.sourceVersion` 选择目录；服务版本与 SDK 发布版本独立，已有测试允许同目录 Provider 3/2、Gateway 4/2。v1 IDL/视图冻结，不存在隐式 v1 adapter。

Rust、TypeScript、Dart 的接收容错与请求严格校验已由统一 IDL 生成；Rust/Dart 的公共范围校验辅助函数使用同一选择规则。Provider 响应区间必须与 descriptor 一致，手机在订阅/写入前验证选择结果。未知事件合法游标推进、未知能力过滤、未知状态阻止发送/自动派发分别有消费层测试。桌面旧 v0 不能表达的状态显式报不可表示，未把未知状态映射为空闲。

校验揭示 RuntimeSelectResponse 过去将探测中的候选作为空版本安装返回。v2 改为 selected=null、pending=候选；完成后仍由 inventoryChanged 发布已验证安装，安装版本的非空约束保留。公开 Schema enum/oneOf 列举已知结构，容错 codec 必须先分类，再对已知分支严格校验；不能把任何解析错误吞成 Unknown。

公开 SDK 导出闭包增加 `$id` 与实际目录版本/相对引用一致性检查。独立作者只读导出 SDK，标准 Ajv 预检以及四包各 19 项通过，未读取 Host。Provider 生命周期、Registry 和三 profile 并发回归保留。实际两端验收范围及产物见 `knowledge/40-runbooks/desktop-android-protocol-joint-test.md`，摘要一致只用于来源验证。
