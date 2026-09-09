# Codex 强制接管的活动与进程保护边界

## 规则

`conversation.resume` 的可选 `force` 省略或为 `false` 时维持原行为。只有显式 `true` 可以请求退出同用户的本机 Codex Desktop；必须先拒绝活动对话，普通 acquire 确认 `conversation_write_conflict` 后才允许关闭。关闭桌面是全应用操作，会影响该桌面的其他任务，客户端必须在明确说明影响后由用户触发。

不得按 `codex` / `ChatGPT` 名称批量终止进程，不得使用递归 `taskkill /T`。Codepet 自身、Provider、它们启动的子进程以及当前 Provider 的祖先必须保留；不能确认目标身份时返回失败。

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
3. 按 Windows `WindowsApps/OpenAI.Codex_*/app/{ChatGPT,Codex}.exe` 或 macOS `Codex.app/Contents/MacOS/Codex` 精确根路径选取同用户进程树。独立 CLI、ChatGPT 应用、其他用户的桌面不在目标内。未支持的布局或平台返回 `force_takeover_failed`。
4. 排除 Codepet/Provider 子树和当前进程祖先；若桌面本身是当前 Codepet 的祖先，拒绝整个关闭操作。目标须有可读取路径、创建时间和用户身份；执行前重新获取进程快照，身份变化、新子进程或保护关系变化均拒绝。Windows 在同一个进程 handle 上校验路径/创建时间并终止，不通过名称或递归命令操作。
5. 先关闭根进程，防止它重新拉起 App Server，再结束已验证的目标后代；等待最多三秒确认桌面及已选进程退出，重启/超时返回失败。退出后重新检查活动，最多十次尝试取得写锁，间隔 100ms。写锁仍被其他 runtime 持有时仍返回 `conversation_write_conflict`，不能把“已发送关闭”当作接管成功。
6. `conversation_active` 表示目标或已知其他 Codex 会话活动；`force_takeover_failed` 表示无法确认、关闭失败或超时；二者放在 resume 的 `interactionError`。`capability_unsupported` 表示 Provider 不支持此行为。后续 history 失败保留 `interactionAcquired=true`，沿用原分阶段结果。

## 来源

本次手机“空闲对话强制接管”需求；既有 `provider.rs::execution_outcome_error` 对官方 active-writer 错误的精确映射；现有双链路隔离决策 `knowledge/50-decisions/codex-remote-and-desktop-companion-dual-channel.md`。进程关闭不更改 Companion 与 Remote 的事件/状态隔离。

## 验证方式

- 进程规划单元测试覆盖同名/同路径 CLI 的保护、Codepet 子树、桌面祖先拒绝、PID/父关系变化、新子进程、其他用户、不可读取路径和终止失败。终止回调为 mock，不操作本机进程。
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
