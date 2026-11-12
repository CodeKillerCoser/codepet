# Observation 能力集合与桌宠接入边界提案

> 已合并为 [Observation 与 SDK 完整改造计划](observation-and-sdk-refactor-plan.md)。本文保留早期调研依据，实施边界和迁移顺序以完整计划为准。

状态：2026-11-11 架构评估，待产品范围与接口确认，未实施。用户暂缓桌面对话，提出接入钉钉 DWS、飞书 CLI、Multica 等来源，支持展示信息、快速回复及激活原应用。

最新范围：用户明确应拆分 observation 或 notification 能力集合，本部分仅接入桌宠。以下以 Observation 为建议名称，将通用通知中心设想收敛为独立能力契约和 Pet 接入；名称及方法未最终定稿，不据此实施。

具体协议拆包、握手、订阅、动作和版本迁移建议见 [Observation 协议提案](observation-protocol-proposal.md)。

## 背景

用户电脑端主要使用原生 App，CodePet 的价值可以集中在跨来源关注、轻量处理和返回原工作环境。通用通知与编程任务在身份、状态和操作上存在差异，不能只给 PetTask 增加几个来源枚举。

## 目标

- 来源通过稳定契约发布信息及可用动作，界面无需理解钉钉、飞书或 Agent 的原生协议。
- 桌宠与顶部面板共享 Pet 侧信息和动作状态；Observation 的订阅生命周期与 Agent 控制生命周期分开。
- 用户能识别来源、账户、会话及操作结果，并按来源控制打扰。

## 非目标

本期只考虑供桌宠消费的能力集合，不建设通用通知列表、全局收件箱或多展示插件平台。不开放任意第三方 UI 代码执行，不开发插件市场，不复制完整 IM 或 Agent 会话客户端，不为全部来源承诺收消息、回复与精确定位能力。

## 现状理解与证据

- `crates/codepet-host/src/pet_gateway/mod.rs` 的 SOURCES 固定为 codex/claude/opencode，订阅名通过 `dev.codepet.{id}` 构造；已有来源启停、健康及原子快照。
- `pet_gateway/projection.rs` 直接识别 Hook 和 OpenCode 原生事件，以 provider/session 归并任务，维护状态和有界去重；缺口使任务变为 unknown。来源语义仍耦合在 Host 投影中。
- `protocol/pet/v1` 有快照与 action 定义；Gateway 未重写默认不支持的 action。`frontend/lib/pet/model.ts` 轮询快照并维护窗口内的 dismiss 状态；当前生产桌宠是只读任务展示。
- `DesktopPet.svelte`、`NotchPet.svelte` 已是两套具体展示，共享 model；Tauri surface 承担原生窗口行为，是分离展示与信息源的基础。
- `src-tauri/src/app/notifications.rs` 的钉钉/飞书实现是旧 PetEvent 体系下的出站机器人策略，不是通用入站订阅与回复连接器；旧 `agent/actions.rs` 也不能作为当前 Pet 快速回复或窗口激活已接通的证据。
- Provider PluginManager 已有进程、通信、生命周期、订阅分发；Provider SDK 的业务模型面向 Agent，不应要求 IM 来源伪造 project/conversation/turn。

外部一手入口：[钉钉 DWS](https://github.com/DingTalk-Real-AI/dingtalk-workspace-cli/blob/main/README_zh.md)、[飞书 CLI](https://github.com/larksuite/cli/blob/main/README.zh.md)、[Multica](https://github.com/multica-ai/multica)。本轮仅核对项目和文档入口，没有验证三者事件订阅、发送身份、原应用深链或实际账号权限。Multica 是对用户拼写 mulitca 的暂定解释。

## 实现路径

建议从现有 Provider 中提取独立 Observation 能力契约，供桌宠通过 Host 订阅。它与 Agent 控制能力可由同一个插件同时提供，也可由仅有 Observation 的插件独立提供；契约和生命周期独立，不要求立刻拆成两份二进制或两套插件宿主。

建议使用 Observation 表达“观察外部应用的信息和活动”；notification 可用于其中的信息事件命名，避免与现有出站机器人通知混淆。Hook、应用插件、CLI 或长连接是能力的具体实现方式，不属于桌宠协议。

能力发现必须允许 observation-only 插件不实现 Agent 实例启动、项目、会话、turn 等接口。Host 不能仅凭插件在线就启动 Agent harness，也不能将观察订阅视为远程控制连接。桌宠按能力发现来源，替换当前固定来源表。

```text
插件的 Observation 能力（Hook / CLI / 原生订阅）
                     ↓ 标准信息、更新和动作描述
          Host Observation 订阅与动作路由
                     ↓
              Pet Gateway → 桌宠 / 顶部面板
                     ↓ 用户动作
          Host → 所属来源插件 / 原生窗口服务
```

Agent 控制仍沿独立 Gateway 服务手机 Remote，与上述路径平行。快速回复、打开来源属于显式可选动作；有观察事件不代表自动具备这些权限或方法。具体动作可以调用插件内部已有实现，但不能因为声明 Observation 就要求启动受控 Agent 或获取写锁。

### 来源与展示契约

先用真实 Agent 事件和一个 IM 来源检验最小契约。信息至少需表达 source/instance/account、原生 item 与 thread 身份、类型、标题摘要、时间、revision、可选状态和动作。普通消息、任务活动、待审批保持不同类型；不能把 IM 消息强制映射为 running/completed。

事件支持新增、原位更新、撤回/失效及断线缺口，Host 的接收顺序与来源版本分别处理。首期按桌宠需要确定有界状态和断线恢复，不把当前 120 条内存任务当作可靠通知收件箱，也不把持久化收件箱设为本期前提。本地关闭一条卡片不自动标记远端已读或删除源消息。

动作由来源声明并绑定不可混淆的目标，包括快速回复、打开来源、标记处理等。Host 按所属来源及账号路由，执行前重新确认可用性和 revision，跟踪请求 ID、执行中、成功、明确失败、结果未知。结果未知不能直接以新 ID 重发；Provider 原生幂等能力不同，不承诺统一 exactly-once。

快速回复要说明以谁的身份发给谁，普通 IM 回复、Agent 继续输入、审批决定保持不同动作语义。只支持打开原应用的来源不显示回复按钮。CLI 只是可选接入手段：接收事件可能需要官方订阅/长连接或受控轮询，发送也受登录身份与权限限制，逐来源验证。

窗口激活分为打开应用、打开具体会话、定位终端/IDE 实例三个能力。来源提供目标引用或深链，Host 的统一原生接口按平台实现打开与聚焦，界面不写系统命令；精确目标不存在时明确报告降级，不能选一个同名窗口宣称成功。动作契约不允许把通知正文当任意 shell 命令执行。

### 插件边界

插件的 Observation 实现负责原生接入、登录和订阅、身份映射、内容标准化与来源动作；Host 负责能力发现、订阅分发和动作路由；Pet Gateway 负责面向桌宠的信息归并与展示状态；展示负责排版、焦点、快速输入及角色动画。

Agent 继续使用已有观察订阅及独立的远程控制 Gateway。通用来源契约不依赖桌宠图集、尺寸或动画，也不复用 Agent 会话协议冒充 IM。可以评估复用 PluginManager 的进程与传输基础设施，但不能在未确认其耦合前宣称现有 Provider 插件已通用化。观察到“等待输入”不等于具有控制权，回复不应暗中抢占原生 App 会话。

## 涉及模块

- Host 的 PetGateway/projection：抽离来源原生解析，向通用信息流适配，保留 Agent 归并的现有语义。
- PluginManager/SDK/协议生成器：评估通用来源 role 和能力描述，复用生命周期基础，避免污染 Agent DTO。
- Host Observation 契约与订阅路由：承接已有 event.subscribe/unsubscribe/notification 的能力拆分；Pet Gateway 持有桌宠所需状态，不新增通用收件箱服务。
- Pet model、两套 UI、回复编辑器及 source 设置：消费统一视图、按动作能力展示控件，保持穿透和焦点规则。
- Tauri 原生服务：按平台处理应用/会话激活，原生窗口引用失效有明确结果。

## 风险与验证路径

- 跨账号/会话误回复：以两账号、相同原生 ID、撤回后操作和过期 revision fixture 验证身份隔离。
- 重连重复通知或丢更新：验证事件重放、乱序、重复、缺口和补偿快照，消息正文更新不重复弹出。
- 通知洪泛：采用关注范围、聚合、静音和优先级；用群消息高频 fixture 验证任务/审批仍可见。
- 回复重复或结果虚假：模拟已发送但回包丢失及来源崩溃；展示未知结果并核对，不盲目重发。
- Agent 观察/控制混用：只读来源不获取控制权；带原生 writer 冲突的真实用例验收。
- 窗口与输入回归：验证应用未运行、多个同名窗口、失效深链、中文输入、焦点退出恢复、点击透传和多显示器。
- 双展示状态分裂：桌宠/顶部面板同步处理、隐藏和恢复，以共享 Pet 状态为准。
- observation-only 插件被当作 Agent：用只实现观察的 fixture 验证可发现、订阅、关闭，且不要求 conversation/turn/instance.start；禁用观察不能停止 Remote 正在执行的任务。

## 测试计划

先用现有 Agent 加一个可验证的 IM 来源跑通新增/更新 → 展示 → 快速回复 → 源端确认 → 激活原应用；能力缺失时诚实降级。再验证观察订阅取消/恢复不影响独立控制链路、有界状态与清理、多源高频事件。按改动运行协议 freshness、Host 路由/状态测试、前端组件测试和真实 Tauri UI 验收。

## 知识沉淀与未知项

本文为提案，未改变既有已交付架构。实施后再更新 Pet 窗口、Provider 活动订阅和产品定位文档。能力命名、manifest/SDK 拆分位置、首个 IM 来源、账号授权和 CLI 版本兼容仍未定；第三方 UI 插件和通用通知中心暂不在范围内。

本轮执行 `git status --short`、相关源码与知识文档静态阅读、官方项目查询、`git diff --check`；仅修改文档，未运行应用测试、连接外部账号或安装插件。
