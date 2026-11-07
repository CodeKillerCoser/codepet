# App Server 空闲回收与任务生命周期

状态：已实现，需要重新构建并安装 Host/Provider 才生效。替代原心跳驱动的回收与此前局部超时补丁。

## 背景与目标

App Server 可能自行打开并持有会话，缺少可靠的逐会话释放接口。结束旧进程可以释放这些锁；重启一个干净的新进程不会自动恢复全部旧会话。因此持续空闲后的回收有价值。2026-11-07 起 Codex 在自动停止旧进程后立即拉起干净的新进程，避免手机在线时长期停留在不可用状态；其他 Provider 默认保持释放策略。

手机在线、失联、卸载或心跳延迟都不决定任务生死。自动释放的必要条件是：整个实例没有未结束任务、待处理请求或未确定结果的执行，并持续空闲达到窗口。

## 非目标

不增加每会话 Server，不重建历史数据库，不强制禁止自动启动，不改变用户明确停止/释放和真正 Host 退出的清理语义，不实现进程崩溃或关机后的任务恢复。

## 实现与职责

- `sdk/rust/codepet-provider-sdk/src/runtime/heartbeat.rs` 只校验 Host session、sequence 和客户端 revision 并返回 pong，不再含实例 worker、过期回收或 start/stop。
- `runtime/activity.rs` 是唯一空闲投影：按实例跟踪 turn、approval、等待输入/运行中的会话摘要、未知发送结果、Ready 与 idleSince。终态墓碑避免晚到 start 响应复活已完成任务；其他会话终态不能解除当前活动。
- `runtime/idle.rs` 每秒复核，只有 Ready 且持续空闲达到窗口才调用已有 instance.stop；启用 restart_after_idle 的 adapter 紧接着调用 instance.start。回收与 mux 请求准入共用执行门，保护尚未返回或排队中的请求。此门目前是 Provider 进程级，其他实例的慢 RPC 可能保守推迟释放，但不会取消其任务。
- `runtime/options.rs` 的 idle_timeout 默认 300 秒，必须大于零，与 20/60 秒心跳参数无关。Codex 可通过进程环境 `CODEPET_PROVIDER_IDLE_TIMEOUT_SECONDS` 调整正整数秒数；其他 SDK 集成可设置 StdioServerOptions.idle_timeout。restart_after_idle 默认 false，Codex 显式启用，不由心跳或客户端数量决定。
- Host manager 在显式启动插件时启动已启用实例，不再把启动责任交给心跳。历史读取与新建会话保留按需恢复，是否有 Remote 在线不再是本地业务读取的前置条件；远程认证仍由原接入层执行。
- Codex 自动回收复用既有 stop/start 和 generation 隔离，不设置手动 release 的持久暂停标志。自动停止后通过同一个 start_runtime 启动；Stopped→Starting→Ready 是短暂过渡，新请求仍需等待 Ready。原有手动 release→保持暂停→显式 resume 行为保留。

## 活动与计时规则

运行、排队、待授权、等待用户输入都算活动。最后一项活动结束才开始完整的空闲窗口。业务 create/acquire/start/steer/approval 操作完成会重置窗口；失败的明确业务请求也给予新的窗口，避免立刻争抢资源。心跳、历史读取、列表轮询和 metadata 更新不续期；读取执行期间仍由执行门保护，不能被中途终止。

Codex 对已经发送但结果未知的 start/steer 保留原错误码，并添加 details.outcome=unknown；SDK 保守保留该会话，由于没有可靠的目标 turn ID，不能用其他任务的终态猜测它已结束，需要显式清理；旧代迟到错误不能污染新代。不把“没响应”当作“没任务”，也不自动重发。

所有会话共用一个 Server，必须全部空闲才能释放。到期回收持有执行门，已准入请求阻止回收；回收先进入则新请求等待旧代退出，不写入即将结束的进程。停止失败重新等待一个空闲窗口再尝试，避免每秒重复清理。自动启动若真正失败，保留错误供显式重试，不在后台无限重启。事件仍是生命周期状态的权威来源，idle start 记录状态 revision，仅在期间没有更新事件时应用返回快照，避免迟到的 Starting 响应覆盖 Ready/Error。

## 释放与恢复

释放清空旧 generation 的 executions、审批和已观察会话集合。新进程启动不遍历旧会话执行 resume。历史读取仍通过原生接口；不因“进程重启”推断为“全部旧锁重新占用”。正常 acquire 只恢复当前请求指定的会话。

Remote 沿用现有 Provider 状态、历史加载和恢复入口。本次未新增协议状态、额外读库或手机续租机制。Host 集成测试覆盖不发送任何 start 请求时 Stopped→Ready 的完整周期，以及之后的历史查询与 resume；实际 App/桌面锁体验还需安装版端到端验收。

## 风险与验证

- 失联误杀：离线运行/审批 binary 测试超过短空闲窗口仍保留，任务可自然完成；SDK 用可控时钟跨越数小时无活动输出。
- 在线不释放：真实 Provider binary 持续正常 ping 和历史读取，仍在空闲期限后退出旧 PID。
- 多会话误回收：A/B 分别完成，最后一个活动结束后重新等待完整窗口。
- 旧锁重占：先加载 A/B，释放并启动新 PID，启动和读取历史不增加 A/B 的 thread/resume；随后只加载 C。
- 准入竞态：执行中请求挡住回收；回收持有门时新请求不能进入；未知结果与迟到错误有独立回归。
- 恢复死路：Host 无手机连接时也能恢复历史和交互；手动 release 的暂停语义有回归。
- SDK 分发：独立导出包含 idle.rs，运行导出目录 cargo check。

## 知识沉淀与边界

同步连接架构、离线执行、writer 生命周期和 SDK 分发指南，统一以业务空闲判断回收。没有在真实用户任务或桌面 Codex App 上操作进程；实际会话锁释放依据用户提供的语义，当前自动化验证的是旧 PID 退出、新 PID 与不自动 resume 的行为。原生 App 锁兼容性需在安装版进一步验证。

## 历史验证记录（2026-11-05）

2026-11-05，基于 `b84689f` 的本地工作分支：

- `cargo test --manifest-path sdk/rust/codepet-provider-sdk/Cargo.toml`：70 通过、5 忽略，包括 6 项独立 idle 测试与已有 mux 请求取消/退出回归。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway`：35 通过；`--test builtin_provider_integration -- --test-threads=1`：3 通过，包含三个内置 Provider、空闲后的历史/交互恢复、手动释放保持暂停。
- Codex `provider_vertical` 的 `idle_` 与 `sdk_heartbeat_` 选择：4 项进程级测试通过。全套一次结果为 58 通过、2 失败、1 忽略；force_takeover 权限错误单独重跑通过，图片 data URI 断言是此前在原始基线复现的失败，未改动该断言。不能宣称全套全绿。
- Claude `--lib`：7 通过；OpenCode `--lib --test provider_vertical`：22 通过、1 个真实 runtime 测试忽略。
- `npm exec --yes --package=bun -- bun test tools/cp-sdk-gen/cp-sdk-gen.test.mjs`：3 通过；独立导出 Provider server SDK 并运行 cargo check 通过。
- `git diff --check` 通过。Cargo 使用外部 target 缓存；Host fixture 中固定 target 路径的临时符号链接已在验证后移除。

验证使用 fixture/独立测试进程，没有停止真实用户任务，也没有实际发布、安装或修改系统上的 Provider。真实 Codex 桌面锁与手机 UI 全链路验收仍未执行。本次仅修改 Host/Provider/SDK 与架构文档，没有更改 Remote 通知实现。

## 2026-11-07 回归

SDK 可控时钟覆盖 opt-in stop→start、显式停止不复活、迟到 Starting 不覆盖 Ready/Error；Codex binary 验证新 PID 自动产生且不 resume 旧会话，离线 running/approval 仍受保护。Host 集成验证自动重启事件、恢复后的列表/历史/交互，以及手动 release 保持停止。与本次连接信息功能的完整检查见 [连接通道与流量展示](connection-channel-and-traffic.md)。
