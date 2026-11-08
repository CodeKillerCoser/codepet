# Host 统一调度 Provider 控制权释放

状态：2026-11-08 已实现；尚未构建、替换安装版或进行真实手机与原生桌面会话验收。

## 背景

此前 SDK 的默认空闲停止策略误作用于 Claude/OpenCode，修复记录见 [默认空闲回收误停其他 Provider](../40-runbooks/provider-idle-policy-opt-in.md)。`ad5c0f6` 先将其改为显式 opt-in。用户随后要求将调度上移 Host，通过 release-control 协议表达释放原生控制权，而不是向全部实例发送 stop。

## 目标与非目标

- Host 统一负责触发、限频和实例生命周期边界，Provider 负责活动安全检查与原生释放实现。
- 当前仅 Codex 参与，空闲释放旧 Server 后启动干净 Server；Claude/OpenCode 保持原有生命周期。
- 保留离线任务、审批、未知发送结果及手动释放后的会话暂停语义。
- 不调整 Remote/Gateway 协议，不更改用户显式 stop，不增加自动恢复旧会话或重发任务的行为。

## 协议与实现路径

`protocol/provider/v1` 新增控制通路方法 `instance.releaseControl`，同时为可选能力。请求为 `{route, idleTimeoutSeconds}`，窗口限 1..86400 整秒。响应只有 `outcome`：

| outcome | 含义 |
| --- | --- |
| released | 旧控制权已释放，干净运行时启动已接受；Ready 由生命周期事件确认 |
| busy | 有任务、待处理交互、未知结果或进行中请求，未执行释放 |
| notIdle | 尚未达到完整连续空闲窗口，未执行释放 |
| unavailable | 当前路由或状态不可释放，包括显式停止后的实例 |
| unsupported | 未声明支持；无副作用，不以 stop 模拟 |

Host 每个插件 generation 启动独立调度任务，持有弱引用，每秒检查启用且 Ready 的支持者。实例首次符合条件后等待完整窗口，默认 300 秒，由 `PluginManagerConfig.release_control_idle_timeout` 配置。每实例最多一个在途调用；busy/notIdle 最多每 5 秒复核，成功和错误等待完整窗口，unsupported 在当前插件代内停用。插件退出、替换、Host shutdown 后任务退出并取消在途 RPC。不同插件不共用操作锁或等待队列。

Provider SDK 不再有定时器。`Activity` 保留最后业务活动、全部会话任务、审批、等待输入、未知发送结果的投影。Host 请求取得非阻塞独占准入门后复核；已准入及排队的普通 RPC 持有共享门，释放不能中途终止它们。只读查询执行期间也受保护，但完成后不重置空闲计时；业务交互重置计时。执行门仍为 Provider 进程级，多实例时保守推迟回收，不取消其他实例任务。

SDK 通过新 trait 方法调用 Codex。Codex 再检查原生执行槽和待审批内容，沿用 stop、generation 清理和唯一 start_runtime 路径。自动释放不设置或解除手动暂停标记；新 Server 不遍历旧会话 resume。普通 Provider 不声明能力，Host 不调用；若误收到请求，SDK 返回 unsupported。使用其他语言/自定义 serving loop 的 Provider 必须自行满足相同原子准入合同。

已准入释放独立于响应流存在，避免超时/取消留下半次回收。`provider.shutdown` 绕过准入门和控制处理许可，先取消执行再关闭 Provider；EOF 同样取消并清理。响应不携带实例快照，避免迟到 Starting 覆盖较新 Ready/Error。成功及错误使用现有年月日、时间、PID、线程、文件行号文本日志，正常空闲检查不重复打印。

## 涉及模块与兼容性

- Provider schema/manifest 与生成 Rust/TypeScript SDK：定义能力、控制通路和严格请求/响应，保持唯一协议源。
- Host manager：调度窗口、单飞、限频和插件代隔离；Gateway 过滤此 Host 专属能力，手机无须配套升级。
- Provider SDK mux/activity/release_control：活动保护、请求取消和退出清理；移除 `idle_timeout`、`restart_after_idle` 选项。
- Codex mapper/provider/main：声明并实现能力，删除 `CODEPET_PROVIDER_IDLE_TIMEOUT_SECONDS` 读取。测试改用 Host 配置或真实协议请求。
- `tools/cp-sdk-gen/provider-runtime.mjs`：分发新手写模块，避免导出的 Provider SDK 缺文件。

Host 与内置 Provider 必须配套更新。旧 Provider 未声明能力时新 Host 跳过，不能假设它的旧自主定时器会消失；旧 Host 也不能保证解析新增 capability。升级时应替换整套 App/Provider 包。

## 风险与验证

| 风险 | 回归验证 |
| --- | --- |
| 活跃任务被停止 | SDK 可控时钟覆盖多会话、排队、审批、待输入、未知结果；Codex 离线执行/自然结束/审批进程测试 |
| 回收与请求争抢 | SDK 共享/独占门竞态测试，真实 mux 验证取消响应仍继续、单控制许可时 shutdown 可中断 |
| 旧会话锁自动重占 | 真实 Codex Provider + fixture 检查旧 PID 退出、新 PID 不新增旧 thread/resume |
| 重复调度、停止后复活 | Host 单飞/限频/停止后重新计时单测；进程测试验证重复请求 notIdle、显式停止 unavailable |
| 其他 Provider 被误回收 | 三个真实 Provider 二进制集成，Codex 连续两轮回收而 Claude/OpenCode 保持 Ready |
| 重启失败或退出挂住 | 初始化失败、初始化卡住时 shutdown 的真实进程测试，验证全部拥有的子进程退出 |
| SDK 分发或协议漂移 | protocol:check、完整 SDK 测试和 cp-sdk-gen 导出测试 |

## 验证记录与未知项

本次已通过协议检查 22 项、Provider SDK 73 项（5 个 fixture 入口按原配置忽略）、Host 调度单测 3 项、manager_gateway 40 项、内置 Provider 集成 4 项、Codex 单测 80 项（1 个本机环境测试按原配置忽略）。13 项独立进程级测试通过，覆盖手动释放、Host 回收、离线任务/审批、失败与关闭；SDK 导出测试 3 项通过，导出目录 cargo check --offline 通过；Tauri bridge 6 项通过。没有运行已知含无关基线失败的整套 Codex vertical suite。

运行命令：

```sh
npm run protocol:check
cargo test --manifest-path sdk/rust/codepet-provider-sdk/Cargo.toml
cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib release_control
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration -- --test-threads=1
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --lib
# provider_vertical 分别运行 host_release_control、sdk_heartbeat_、release_interaction、provider_has_no_autonomous_idle_timer 过滤项
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --test provider_vertical host_release_control -- --test-threads=1
npm exec --yes --package=bun -- bun test tools/cp-sdk-gen/cp-sdk-gen.test.mjs
npm exec --yes --package=bun -- bun tools/cp-sdk-gen/cp-sdk-gen.mjs --package provider --role server --lang rust --output /tmp/codepet-release-control-sdk-export
cargo check --manifest-path /tmp/codepet-release-control-sdk-export/codepet-provider-sdk/Cargo.toml --offline
TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo test --manifest-path src-tauri/Cargo.toml --lib runtime_gateway::tauri_bridge::tests
git diff --check
```

尚未在安装版验证原生 Codex 桌面锁与手机 UI 全链路；fixture 的 PID/请求证据不能替代该验收。规约同步至 [心跳与任务生命周期](../60-rules/heartbeat-task-retention.md)；旧 SDK opt-in 文档保留为历史修复证据。
