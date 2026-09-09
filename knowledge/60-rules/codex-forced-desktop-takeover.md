# Codex 强制接管的 harness 标记与进程保护边界

## 规则

`conversation.resume` 的可选 `force` 省略或为 `false` 时维持原行为。只有显式 `true` 可以请求结束同用户的未标记 Codex 进程；必须先拒绝活动对话，普通 acquire 确认 `conversation_write_conflict` 后才允许关闭。该操作包括桌面应用和独立 CLI，会影响其他 Codex 任务，客户端必须在明确说明影响后由用户触发。

先枚举 Codex 进程，再检查 harness 环境标记；不得跳过标记检查按名称直接终止，不得使用递归 `taskkill /T`。Codepet 自身、Provider、它们启动的子进程以及当前 Provider 的祖先必须保留；不能确认目标身份时返回失败。

## 适用场景

- `protocol/gateway/v1/schema.json`：Remote 的 `ConversationResumeRequest.force?: boolean`。返回结构保持 `interactionAcquired / interaction / interactionError / history / historyError`；接管失败不返回历史成功，也不宣称已经获得交互权。
- `crates/codepet-host/src/gateway.rs`：将显式 force 传入 Provider `ConversationAcquireInteractionRequest.force`，普通 acquire 传 `None`，普通 resume 继续原路径。
- `crates/providers/codepet-provider-codex/src/provider.rs`：在长期 App Server session 上读取活动、验证冲突、关闭桌面、重新获取写锁，复用已有 configuration 和历史读取逻辑，不重启 Codepet 的 App Server。
- `crates/providers/codepet-provider-codex/src/desktop_takeover.rs`：将纯进程规划/重验证与操作系统关闭操作分离。Claude/OpenCode 明确拒绝 force，避免静默忽略会造成客户端误判的参数。

## 反例

2026-09-09 本机只读进程检查发现，桌面主程序位于 `WindowsApps/OpenAI.Codex_*/app/ChatGPT.exe`，其 CLI 子进程为 `AppData/Local/OpenAI/Codex/bin/<version>/codex.exe`。Codepet 也可使用相同 CLI 文件。仅比较进程名称或 CLI 可执行路径，会误伤 Codepet 自己的运行时；把 `ChatGPT.exe` 名称当作 Codex，也会误伤独立 ChatGPT 应用。

## 推荐做法

1. 校验活动：读取 thread/read 完整 turns，拒绝 active 状态、inProgress turn、等待审批/输入，以及系统错误或不可读取的活动信息。额外拒绝本 Provider 已知的任意活动 Hook 会话或活动 execution slot，因为退出桌面影响整个应用。进程内读写锁阻止 force 与正常 turn 操作同时执行。
2. 普通 acquire 已成功则复核活动并直接返回，不关闭任何桌面。仅标准写入冲突进入关闭路径，其他错误保持原错误。
3. `client.rs::codex_app_server_command` 在启动 harness 前显式注入 `CODEPET_CODEX_HARNESS=<provider PID>`，覆盖继承值；子进程继承此标记。标记非空就保护，不要求 PID 等于当前 Provider，因此其他 Provider 实例创建的 harness 也不会被误杀。标记仅用于进程归属保护，不是安全认证凭证。
4. 先枚举同用户的 `codex` / `codex.exe`，并识别 Windows `WindowsApps/OpenAI.Codex_*/app/{ChatGPT,Codex}.exe` 的桌面根进程；只有环境可读取、且不含非空标记的候选才可结束。不按桌面根进程扩大到所有后代；普通工具进程不在目标内。读取环境只针对候选，绝不输出其他环境变量的值。环境为空或不可读取一律视为未知，拒绝此次关闭。
5. 额外保留 Codepet/Provider 子树及当前进程祖先，保护升级前未注入标记的自有 harness；带标记的孤儿或重挂父进程的 harness 也保留。执行前重新枚举并检查身份/标记，任一候选变化即中止。Windows 使用同一个进程 handle 检查路径/创建时间并结束；退出后重新扫描未标记 Codex，最多等待三秒，再按既有流程复核活动、最多十次尝试写锁。找不到待关闭进程时直接重试 acquire，不因没有桌面根进程而拒绝独立 CLI 的接管场景。
6. `conversation_active` 表示目标或已知其他 Codex 会话活动；`force_takeover_failed` 表示无法确认、关闭失败或超时；二者放在 resume 的 `interactionError`。`capability_unsupported` 表示 Provider 不支持此行为。后续 history 失败保留 `interactionAcquired=true`，沿用原分阶段结果。

## 来源

本次手机“空闲对话强制接管”需求；既有 `provider.rs::execution_outcome_error` 对官方 active-writer 错误的精确映射；现有双链路隔离决策 `knowledge/50-decisions/codex-remote-and-desktop-companion-dual-channel.md`。进程关闭不更改 Companion 与 Remote 的事件/状态隔离。

## 验证方式

- 进程规划单元测试覆盖同名/同路径 CLI 的保护、Codepet 子树、祖先保护、PID/父关系变化、新候选进程、其他用户、不可读取环境和终止失败。终止回调为 mock，不操作本机进程。
- Provider fixture 集成测试覆盖 force 省略/false 的冲突、true 后成功、重复 true 不重复关闭、保留同一 App Server PID、活动/审批/输入拒绝、关闭失败和关闭后活动。关闭回调只写临时 fixture 标记。
- Gateway 集成测试验证 force 转发及活动错误保留在分阶段结果；协议生成检查和协议测试验证 Rust/TypeScript/Dart 输出一致。Remote SDK 用 `bun tools/cp-sdk-gen/cp-sdk-gen.mjs --package gateway --role client --lang dart` 生成并同步 lock，禁止手改生成文件。
- 本任务未执行真实桌面关闭。macOS 的实际权限/退出行为及 Windows Job Object 引发的间接子进程退出仍需在隔离环境验证。macOS 使用重新校验后向单个 PID 发信号，不能提供 Windows handle 相同的原子进程身份保证；不得将本次 Windows mock/fixture 结果表述为 macOS 实机保证。
- 外部 Desktop 可以在最后一次活动读取后启动新任务；现有跨 runtime 协议不提供“检查空闲并关闭”的原子操作。多次检查和本进程锁缩小窗口，但不保证外部客户端之间的原子互斥。新增原子 shutdown API 时应替换此边界，而不是放宽活动校验。

## 本次验证记录

首次使用空 target 的 Rust 测试被 Windows 应用程序控制策略阻止运行 getrandom build script（os error 4551），未更改系统策略。正常复用已有 `D:/17633/Documents/Code/codepet/crates/target` 构建缓存后完成以下验证；该缓存目录不包含源码改动。以下 Cargo 命令均使用此 `--target-dir`：

- `cargo check --manifest-path crates/Cargo.toml -p codepet-provider-codex -p codepet-provider-claude -p codepet-provider-opencode -p codepet-host --tests`：通过。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --lib --test provider_vertical`：75 单元通过（含 6 项进程 mock）；52 集成通过（含 2 项 force fixture），1 项真实 CLI smoke 忽略，1 项既有图片断言失败。
- 失败为 `provider_v1_round_trips_fixture_app_server_lifecycle_and_approval` 的 `!fetched_json.contains("data:image/png")`。从 `git archive 09e2fe8 crates sdk` 产生的独立原始源码执行该精确用例，复现同一断言失败；未将其归因于强制接管，也未改动该历史图片行为。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway gateway_resume`：1 项通过，内部同时验证 force 省略/false/true 和活动拒绝。
- `node tools/protocol-codegen/generate.mjs --check` 与 `node --test tools/protocol-codegen/test.mjs`：生成输出一致，20 项通过。
- Remote 输出的 `bun tools/cp-sdk-gen/cp-sdk-gen.mjs --package gateway --role client --lang dart --output <remote>/sdk/gateway --check`：通过。


### 环境标记修订验证

用户纠正关闭范围后，增加独立 CLI 应被选择、重挂父进程的带标记 harness 保留、标记变化终止前拒绝、环境不可读取拒绝的 mock 断言。命令构建测试检查注入值等于 Provider PID；force fixture 启动时检查实际继承到非空标记。此前将独立 CLI 排除的结论已被本修订替换；没有执行真实本机终止验证。

修订后执行 `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --lib --target-dir D:/17633/Documents/Code/codepet/crates/target`：76 项通过；同 target 下 `--test provider_vertical force_takeover`：2 项通过（包括实际 fixture 子进程接收到标记）。`git diff --check` 通过。
