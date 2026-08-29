# Codex Desktop 私有 IPC Provider

> 当前状态（2026-08-29）：本页文件名为保持知识链接兼容暂时保留。Codex Provider 的运行时数据源已经从 Code Pet 独立启动的 `codex app-server --listen stdio://` 改为当前用户正在运行的 Codex Desktop Owner/Follower 私有 IPC。该协议没有兼容性承诺，接入必须版本耦合并 fail closed。

## 背景

Code Pet 需要展示 Codex Desktop 中已经存在的任务及其状态变化。独立启动 App Server 会形成另一个运行实例，不能把它收到的数据当成 Desktop 当前任务的权威状态，也可能把回复或审批发往错误实例。因此阶段一只附着 Desktop 的私有 IPC，不再启动独立 App Server，也不使用 Hook、transcript、audit 扫描或文件监听补齐任务状态。

## 目标

- 安全连接当前用户的 Codex Desktop IPC，并把连接失败公开为可诊断的 Provider unavailable。
- 在已知 thread id 的前提下完成 owner discovery、following、完整历史和状态快照 bootstrap。
- 严格按 revision 应用 snapshot/patch，并把可判定状态映射到 Code Pet Standard Protocol。
- 让桌宠展示运行、等待审批或输入、完成、失败和中断等状态。
- 保持私有 DTO、路由字段和生命周期消息在 Codex Desktop adapter 内部。

## 非目标

- 不把私有 IPC 扩展成 Code Pet 的公共协议，也不让 UI 解析原生 payload。
- 不通过 transcript、Hook、audit、文件监听或目录扫描发现任务。
- 不在 Desktop IPC 失败时启动独立 App Server 或 Codex CLI 作为回退。
- 阶段一不发送回复、快捷回复、审批决定或其他写操作。
- 不承诺在完全未知 thread id 时枚举 Desktop 中的全部任务。

## 已验证证据

- 当前本机 Codex Desktop 默认使用 `~/.codex/ipc/ipc.sock`；目标是当前有效用户所有、权限为 `0600` 的 Unix domain socket。
- 每个 IPC frame 使用 4 字节 little-endian 长度前缀，后跟 JSON；bundle frame 上限为 256 MiB。
- 连接初始化需要唯一 `clientId`。消息带有目标路由，多客户端不会天然共享同一条流，因此接收方必须过滤不属于自己的定向消息，同时分发明确的广播消息。
- 当前 Owner/Follower 协议族为 v1，状态广播实现当前为 v11。这是私有实现版本，不是 Code Pet Standard Protocol 版本。
- 已知 thread id 时，可依次执行 owner discovery、设置 `following=true`、接收初始 snapshot、请求完整历史并取得 revision，之后等待对应 revision 的 snapshot，再进入增量跟随。
- patch 只有在 revision 连续时才能应用。遇到缺口必须重新请求完整 snapshot；等待期间不得把后续 patch 拼接到旧状态。
- 断线后旧 revision、following 和未完成请求都不再可信。客户端需要清空连接态，并以有界退避重新连接和 bootstrap。
- 当前私有 IPC 没有可用的任务目录。Code Pet 在完全没有 thread id 的情况下无法从零枚举任务；这项限制不能用 transcript 扫描绕过。
- 2026-08-29 只读联调中，显式提供一个正在运行的 thread id 后，新的唯一 client 成功完成 owner discovery、following、初始 snapshot、完整历史和目标 revision snapshot；同一环境下，另一个 known-set 为空的新 client 在有界观察期内没有收到当前任务公告。因此 `following=true` 公告只能作为机会性发现信号，不能当作任务目录或晚加入保证。

这些事实来自当前 Codex Desktop 打包实现和只读多客户端探针。升级 Desktop 后必须重新核对，不能仅凭历史字段或方法名继续声明兼容。

## 现状理解

Codex Desktop 是任务 owner，Code Pet 是 follower。IPC transport 只负责安全 framing、初始化、请求关联、路由过滤、广播分发、关闭和重连；Owner/Follower adapter 负责私有生命周期和 revision 状态机；mapper 只把经过验证的 thread/turn 语义投影到标准 Provider、Conversation、TurnTask 和 ProtocolEvent。

Code Pet 可以可靠处理两类 thread：连接期间由其他 Desktop follower 明确公告的 thread，以及外部已经提供已知 id、随后完成 bootstrap 的 thread。应用重启后如果既没有受控的已知 thread id，也收不到新的 follower 公告，则不能声称已经发现 Desktop 中所有正在运行的任务。

## 实现路径

### Socket 与 frame 安全

连接前解析 Desktop IPC 位置，并通过不跟随替换目标的 metadata 检查确认目标是 Unix socket、由当前有效用户所有且权限符合 `0600` 安全边界。目标缺失、类型错误、owner 不匹配、权限过宽或连接被拒绝时，Provider 保持 unavailable，并公开不含原始私有 payload 的诊断原因。

frame decoder 必须先校验 4 字节长度，再分配和读取 payload。长度超过 256 MiB、短帧、非法 UTF-8/JSON 或 envelope 不完整都属于协议错误；不得继续猜测边界或复用半帧数据。

### 初始化、路由与请求关联

每次物理连接生成唯一 client identity，并完成 initialize 后才进入可用状态。请求使用连接内唯一 id 关联 response；断线或关闭时所有 pending request 必须以显式错误结束。

接收路径先判断目标路由。发给其他 client 的消息必须忽略，明确发给当前 client 的消息进入请求或 follower 分发，广播消息进入受控广播分发。未知消息只记录有限诊断，不得把原始 JSON 放进 Standard Protocol extension 或 UI。

### Owner/Follower bootstrap

对已知 thread id，adapter 按当前真实生命周期完成 owner discovery 和 following，不能只订阅未来事件。完整历史请求返回的 revision 是 bootstrap 的同步锚点；在收到匹配的完整 snapshot 前，该 thread 不得标记为已同步。

若 owner 不存在、following 被拒绝、历史加载失败或等待 snapshot 超时，应保留明确的 thread/provider 诊断状态，不构造看似完整的会话。由于没有任务目录，空发现结果只能解释为“当前没有收到可附着任务公告，也没有显式已知 id”，不能解释为 Desktop 中一定没有任务。

### Revision 状态机

完整 snapshot 可以建立新的基线。连续 patch 只能从当前 revision 前进一个协议允许的步长；重复或旧 revision 安全忽略，前向缺口进入 `awaiting snapshot` 状态并触发重新同步。在重新建立基线前不对外发布由缺口 patch 推导出的状态。

重连必须清空 revision 基线、following 状态和旧 owner 路由，再从 initialize 与 bootstrap 开始。退避应有上限，并在 Desktop 恢复后允许 Provider 从 connecting/unavailable 回到 ready。

### 标准状态与能力

adapter 只输出 Code Pet 已能证明的公共语义。当前映射至少覆盖 running、waiting approval/input、completed、failed 和 interrupted；等待输入可以复用标准等待状态并保留安全的展示文案，但不得因此暴露审批按钮。

阶段一 Provider 是只读观察者。capability 只能列出已实现的发现/读取方法；`quickReplies` 为空，审批、回复、steer、interrupt 及创建会话等未接通写操作不得声明可用。对应 Gateway 调用必须返回明确的 unsupported，而不是发送私有 follower 请求后猜测成功。

## 涉及模块

- `src-tauri/src/agent/codex_desktop_ipc/transport.rs`：socket 校验、frame codec、initialize transport、关闭和连接边界。
- `src-tauri/src/agent/codex_desktop_ipc/protocol.rs`：私有 envelope、请求关联和路由过滤。它与同目录模块是私有 wire DTO 的唯一归属。
- `src-tauri/src/agent/codex_desktop_ipc/state.rs`：thread 附着状态、snapshot/patch 和 revision 状态机。
- Codex Provider mapper：私有 follower state 到 Standard Protocol DTO/event 的单向投影。
- `src-tauri/src/runtime_gateway/`：Provider registry、标准 capability、事件序列和 Tauri transport；不得出现 Desktop 私有方法名或 DTO。
- `frontend/lib/runtimeGatewayActivity.ts` 与 `frontend/PetApp.svelte`：只消费标准状态和 capability，展示任务与 Provider 诊断。
- `src-tauri/src/agent/runtime.rs` 与运行时设置 UI：可以保留 executable 检测，但其结果不决定 Desktop IPC Provider 是否可用。

## 风险

- 私有协议版本漂移：每次升级 Codex Desktop 后复核握手、Owner/Follower 和状态版本；不兼容时 Provider fail closed。
- 路由过滤错误导致串流：用多个唯一 client id 的 fixture 验证定向隔离和广播分发。
- revision 缺口产生错误任务状态：状态机测试验证缺口后停止应用 patch，直到完整 snapshot 建立新基线。
- Code Pet 晚加入但没有 thread id：`following=true` 公告只做机会性发现；通过日志和 UI 明确展示发现范围限制，不扫描 transcript 冒充目录。
- 重连沿用旧连接状态：断线测试验证 pending request、revision、following 和 owner route 全部复位。
- UI 暴露未接通动作：后端 capability 精确测试与前端能力测试共同验证审批、回复和快捷回复不可见。

## 测试计划

- frame codec：little-endian 往返、短帧、非法 JSON 和 256 MiB 上限。
- 路由：当前 client 定向消息、其他 client 消息和广播消息互不串线。
- bootstrap：已知 thread id 在任务已运行时完成 discovery、following、完整历史和 snapshot 同步。
- revision：顺序 patch、重复 patch、缺口重新请求 snapshot，以及等待期间不发布错误状态。
- 生命周期：干净关闭、断线 pending request 失败、有界重连和状态复位。
- mapper：running、waiting approval/input、completed、failed、interrupted 和未知状态。
- capability/UI：阶段一不显示审批、回复或快捷回复；Provider unavailable 展示安全诊断原因。
- 静态检查：生产代码中不再存在 `app-server --listen stdio://` 启动路径，Desktop 私有方法名不出现在 adapter 之外。

## 知识沉淀

- 架构选择见 `../../50-decisions/codex-desktop-ipc-as-provider-source.md`。
- 连接故障见 `../../40-runbooks/codex-desktop-ipc-unavailable.md`。
- 长期约束见 `../../60-rules/codex-desktop-ipc-fail-closed.md`。

## 未知项

- 当前 IPC 没有公开任务目录；无已知 thread id 的完整晚加入枚举仍被协议阻挡。
- Owner/Follower 和状态广播版本没有长期兼容承诺，支持的 Desktop 版本范围仍需随发布验证确定。
- 回复、审批、用户输入、steer、interrupt 和其他写操作需要分别验证请求路由、结果确认和失败语义，阶段一不作支持承诺。
