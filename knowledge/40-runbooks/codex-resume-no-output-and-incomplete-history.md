# Codex resume 后无输出与历史不全排查

## 现象

2026-09-06，Remote 手机选择已有会话、发送消息后，看起来 Codex 没有工作；同时用户报告“拉对话不全”。用户补充确认：第一次打开项目能看到“资源挂载”，重新打开项目后它消失；几个已有会话发送后没有可见消息更新。项目列表缺项与消息更新必须分开检查。

## 复现路径

手机连接 Host，选择已有 Codex 会话，resume 后发送消息。本次对照两个真实会话：

- “查看文档中的资源挂载方案”：`01a072d0-6f4c-7331-98a0-b03f14faa5cf`。
- codepet-remote 下的 hello：`01a072fa-95d8-7770-88cc-3cb2305e5533`。

本次仅读取 Host 日志、原生 SQLite/rollout，并运行独立 App Server 的只读历史探针；没有替用户重新发送消息或获取会话 writer。ADB devices 没有连接设备；随后用户提供手机导出日志，已用于按 traceId 对齐事件。仍没有现场 UI 录像和明确 APK 构建版本。

## 证据

时间均为北京时间。Host 日志为 `~/Library/Application Support/code-pet/logs/code-pet.log`；原生证据来自 `~/.codex/logs_2.sqlite`（只读连接，按 thread_id 查询）和 `state_5.sqlite` 指向的 rollout。排查不复制消息正文、工具输入或 credential。

| 会话 | Host turn.start 成功 / 原生 task_started | 原生后续证据 |
| --- | --- | --- |
| 资源挂载方案 | 04:33:57.336 / 04:33:57.335 | 模型请求超时重试 5 次；04:35:36 回退 HTTP；04:35:42.964 写入 assistant message，随后有多次工具调用 |
| hello | 04:34:39.623 / 04:34:39.623 | 模型请求超时重试 5 次；04:36:27 回退 HTTP；04:36:31.894 写入 assistant message；04:36:32.781 task_complete |

对应 turn ID 分别为 `01a07347-244e-7853-b4d6-493f1561d318` 和 `01a07347-c96f-7ea3-b30c-fb012082f9af`。hello 从启动到完成约 113 秒。资源挂载方案在检查时没有该轮 task_complete，不能声明该轮完整成功。

独立只读探针对两个本地版本分别执行 initialize、thread/turns/list(itemsView=full, limit=20, sortDirection=desc)，不调用 resume/start：

- `/opt/homebrew/bin/codex`：0.151.0。
- `/Applications/ChatGPT.app/Contents/Resources/codex`：0.153.4。
- 两版本结果一致：资源挂载方案 18 轮、70 个 item、nextCursor=null；hello 5 轮、14 个 item、nextCursor=null。
- 用户/assistant item 数分别为 18/23 和 5/7；assistant 数与 rollout 一致。rollout 还包含上下文注入消息，不能用全部 response_item 条数直接认定聊天丢失。
- 探针位于 `/tmp/codepet-resume-investigation/`，仅为临时工具；这些结果不等于生产 Host 响应或手机显示已验证完整。

## 手机日志与后续原生证据

用户提供 `~/Downloads/未命名文件夹 2/codepet-logs-2026-09-05T20-50-06-110792Z/codepet.log`。导出日志为 UTC，下面表格已换算为北京时间。与 Host 同一次 send 的完成时间相比，手机时钟约早 0.6 秒，跨端耗时应优先按 turn/trace 身份关联。

| 手机证据 | 内容 |
| --- | --- |
| 04:33:56，行 3658–3666 附近 | 资源挂载 send accepted，收到 turn.upserted |
| 04:35:40.579，行 3729 | 同 send trace `51f4dd1c853099dc9928cba20a137e82` 收到首个 turn.outputDelta |
| 04:34:39，行 3689–3696 附近 | hello send accepted，收到 turn.upserted |
| 04:36:30.994，行 3783 | hello trace `6cd2b7b2227473d86e8515b07362d3fc` 收到首个 turn.outputDelta |
| 04:36:32.262，行 3794 | hello 收到后续 turn.upserted，原生约同时 task_complete |
| 04:37:05、04:42:00、04:47:03 | 资源挂载历史一直是 70 item；分页 hasNextCursor=false，组装和安装成功 |
| 04:40:44.863，行 4013 | 另一个会话 `01a07311-59d5-7790-8c93-5d50cb40aa57` 收到终态后刷新，仍安装旧的 45 item |

资源挂载第一次 send 的 trace 下共收到 17 个 itemUpserted；hello 收到 4 个 itemUpserted。输出增量日志按每 turn 首条采样，计数 1 不代表只有一个增量。没有对应 first_output.rendered 不能直接证明事件被丢弃：用户切换页面也会导致没有该页面的渲染记录。另一个会话在 04:41:05 有明确 projected/rendered 记录，故并非所有续聊都切断订阅。

原生 `logs_2.sqlite` 只读查询发现：

- 资源挂载：04:42:12 至 04:49:56，24 条 live_writer 历史投影错误，`expected ordinal 294, got 293`。
- `01a07311-59d5-7790-8c93-5d50cb40aa57`：04:31:48 至 04:49:15，71 条同类错误，`expected ordinal 183, got 182`。
- hello 没查到该错误，不能把它也归为历史投影故障。

只读查询 `thread_history_1.sqlite` 并对照 rollout：资源挂载文件已 309 行，投影停在 ordinal 294，历史表仍 18 turn / 70 item；rollout 已含 04:47:09 新 turn、04:49:55 assistant 和 04:49:56 complete。另一会话文件 238 行，投影停在 ordinal 183，历史表仍 9 turn / 45 item。hello 文件与投影均 72，历史 5 turn / 14 item。此差异证明新内容写进了 rollout，但部分会话未进入历史投影，不能把返回旧历史解释为手机漏拉下一页。

## 重复 ordinal 的文件级证据

进一步检查的是 JSONL 内真实 `ordinal` 字段，不是物理行号：

| 会话 | 物理行 | 时间（北京时间） | 类型 | ordinal |
| --- | --- | --- | --- | --- |
| 资源挂载 | 294 | 04:36:58.787 | token_count | 293 |
| 资源挂载 | 295 | 04:42:12.648 | thread_settings_applied | 293 |
| 另一故障会话 | 183 | 04:16:39.099 | token_count | 182 |
| 另一故障会话 | 184 | 04:31:48.445 | thread_settings_applied | 182 |

即恢复后的写入复用了上一条已落盘记录的序号。重复行正好位于投影保存的 next_rollout_byte_offset（资源挂载 1117818，另一会话 1102107），与 expected/got 错误一致。对应首次报错进程为 PID 5631、2330；同进程模型缓存日志的 client_version 均为 0.153.4。

上游当前 [ordinal.rs](https://github.com/openai/codex/blob/main/codex-rs/rollout/src/ordinal.rs) 从文件尾部找到最后一条可解析记录，以 ordinal+1 恢复写入序号；扫描会跳过 Rejected 记录。[历史投影器](https://github.com/openai/codex/blob/main/codex-rs/thread-store/src/local/thread_history_materialization.rs) 遇到小于 expected 的 ordinal 会拒绝继续；[live_writer](https://github.com/openai/codex/blob/main/codex-rs/thread-store/src/local/live_writer.rs) 先写原始记录，再尝试投影，投影失败记警告，因此原始文件还能增长。

这些源码解释了错误机制，但 main 不等于本机二进制的构建源码。尚无证据断定本次尾记录是因解析失败被跳过，还是恢复期间的其他生命周期/并发问题。按两个报错进程搜索 parse/reject/invalid type/rollout line 日志没有命中，不能把“尾扫描跳过 token_count”写成已证实根因。

## 已定位机制与影响范围

### 项目列表消失

本机 `state_5.sqlite.threads.project_id` 中资源挂载为 null；`.codex-global-state.json` 仍有 local legacy assignment。`provider.rs` 的 list_codex_conversations 会 decorate 缺失项目，但 conversation_get、conversation_upsert_event、conversation_status_upsert_event 不执行该补齐。resume/status 会发出无项目的 conversation.upserted。

Remote `device_session.dart::_upsertEventConversation` 按事件顺序接受新 metadata，只保留 readState 和单调时间，project 会被 null 覆盖。项目页复用已加载缓存，故先由 list 得到该会话，随后更新清空归属，重新进入项目时消失。hello 的 native project_id 非空，不具备这个前提。

用户要求先核对原生与包装链，再在 UI 对运行时缺值采用合并。本次按此落地：Remote 仅为缺少非空 project 的运行时 metadata 保留同会话的已知 project；标题、状态、执行配置仍接受新值，非空新 project 正常覆盖。列表合并没有改成此 fallback，显式项目删除仍走既有删除路径。当前 DTO 没有字段 presence 标志，missing 与 null 在解码后无法区分；若未来要通过 runtime event 表达“显式移出项目”，应增加明确语义，不能再借同一 null 表达。

### 新消息不更新或刷新后消失

网络重试解释首输出约两分钟延迟；手机 trace 证明实时事件已到达，不能将“始终没更新”只归因于网络。

原生历史投影 ordinal 错误让 conversation.get 持续返回旧内容。修复前 Remote `models.dart::installCommittedSnapshot` 仅保留缺失的 pending 用户占位，对已收到的 canonical assistant item 直接使用 snapshot 替换；同时丢弃 completedTurnId 对应 liveOutput。这样旧快照会清除已经通过事件收到的新回复，重新打开页面也只能拿到旧历史。

修复前的临时 Dart 最小复现调用当时真实模型代码（该替换方法已随本次修复删除，旧脚本不再适用于新模型）：已有 1 条新 assistant 回复 + terminal turn，安装缺少该 turn 内容的旧快照后得到 0 条消息，terminal turn 仍在。执行命令：`dart --packages=.dart_tool/package_config.json /tmp/codepet-resume-investigation/stale_snapshot.dart`（cwd 为 codepet-remote），输出 `Before stale terminal snapshot: 1 message; after: 0 message; terminal retained: 1`。这证明覆盖机制，不代表已证明用户每一次停留页面时的渲染细节。

最终按用户确认的数据源逻辑修复：取消终态历史查询，实时事件直接更新消息；首次只取最近一页，旧消息仅手动分页前插。不得自动重发 turn。原生投影故障的修复需要先确认运行版本与投影恢复机制，不能直接删库或改 ordinal。

## 验证结果与已排除方向

- 已对齐手机 send trace、Host turn.start、原生 turn/rollout 和 SQLite 投影。
- `flutter test test/conversations/conversation_detail_screen_test.dart --plain-name 'buffers live output while the combined resume is in flight'`：1 项通过，当前源码的合并 resume 缓冲路径未复现普遍丢事件。
- Dart 旧快照覆盖复现成功；此前只做诊断；本轮已修改 Remote 的项目合并逻辑，未修改配置、原生数据库或运行中的 Host。
- 排除“所有发送都没到 Codex”“手机完全收不到事件”“70/45 条只是没加载第二页”。
- turn_start 仍有全量 thread_read 的潜在大历史风险，但它没有阻止上述成功受理的 turn，不作为本次根因。

## 下一步排查与未知项

1. 项目归属统一及旧快照保护是两个独立修复点，各自应有回归测试；UI 验证覆盖重新进入项目、等待首输出、任务终态保留输出、缓存复用与淘汰后再次打开详情。
2. 已证实恢复写入产生重复 ordinal；内部为何错误恢复计数尚未确认，不能凭错误文本认定多 writer。保留 rollout 和投影证据后再研究受支持的恢复方式。
3. hello 已收到输出和终态且历史投影正常；其等待期有明确网络超时，但用户看到完全无更新的具体页面状态仍未被这些日志完整解释。
4. 部分手机区间还出现 WebSocket 心跳超时/关闭，应区分页面切换、应用挂起和原生执行，不能把所有现象合成单一根因。

## 原生 resume 字段核对与项目合并修复

使用本机 0.153.4 binary，在隔离 CODEX_HOME 中对 SQLite backup 和目标 rollout 副本执行 initialize、thread/resume(excludeTurns=true)、thread/read(includeTurns=false)，没有发送 turn/start。副本用于避免占用真实会话 writer；项目路径或文件内容没有输出到诊断摘要。临时脚本为 `/tmp/codepet-resume-investigation/check_resume_project.py`。

| 会话 | 原生 thread/resume | 原生 thread/read |
| --- | --- | --- |
| 资源挂载 | thread.projectId 字段存在，值 null | 字段存在，值 null |
| hello | thread.projectId 为 01a04f09-d723-7562-9ba5-8e99ad683ebe | 同一非空值 |

链路核对：原生 CodexThread 保留 project_id；Provider acquireInteraction 只从 resume 读取执行配置，返回 selection，不返回 Conversation。Host conversation.resume 的 history 是独立调用 conversation.get 取得，Provider get 的 metadata 来自 thread/read。Provider mapper 将非空 project_id 转为 project 资源；Host sanitize_provider_conversation 只清 read_state；Dart GeneratedGatewayMapper 将 project 原样映射。未发现中间链路丢弃非空项目 ID；不能把 Host history.conversation 直接称为原生 resume.thread。

Remote 改动：

- `lib/core/domain/models.dart::mergeRuntimeMetadata`：只为同身份会话的空 project 使用已知值，其他字段使用新值，避免跨会话借用归属；详情事件投影使用该合并。
- `lib/application/sessions/device_session.dart::_upsertEventConversation`：运行时事件合并，防止缓存归属清空导致项目页缺项。
- `lib/application/conversations/conversation_detail_controller.dart`：resume/get 快照及 session metadata 更新采用相同规则，避免列表保留而详情清空。

验证：新增项目重新进入/新标题/较旧时间戳/非空新项目覆盖测试，修改前报 No element（原项目列表为空），修改后通过；resume 详情与后续空项目事件保留归属的 widget/controller 测试通过。`flutter test --no-pub test/devices/device_session_test.dart test/conversations/conversation_detail_screen_test.dart test/gateway/models_test.dart` 共 102 项通过。未调用格式化工具，未打包或安装 APK。

用户补充会话执行一半后重启后台，与历史现场一致：故障边界前没有正常完成该轮，尾部最后记录为 token_count；后台恢复后第一条设置记录重复用最后的 ordinal。旧投影进度与重复行开始字节位置准确一致，说明投影拒绝重复数据，而不是单纯缺一次刷新。此前原生日志的不同 PID resume 记录也支持运行时重建；不能仅据此进一步断定重复序号由哪个内部恢复分支造成。原始文件中已存在重复值，单纯删缓存或重建数据库无法保证修复，尚未改动它们。

## 为什么手机没有收到序号错误

原生当前 `live_writer.rs::write_and_project` 在 durable_write 成功后尝试 SQLite 投影；投影失败只 warn，函数仍可返回 Ok。现场同时出现该 warning、turn.start 成功和实时输出，符合此机制。这里失败的是历史投影，不能把它等同于模型执行必然失败。

CodePet Provider `provider.rs::turn_start` 将成功的原生 turn/start 映射为 accepted=true；这只是受理结果。`client/framing.rs::drain_stderr` 只转发诊断文本，不生成会话错误事件；原现场序号错误证据来自 Codex logs SQLite；后续隔离实测已确认相同错误会输出到 stderr（见下节）。Host 不从原生 SQLite 收集这些 warning。手机后续 get 返回旧但形状合法的数据，也没有协议 error，因而既不会进入发送异常分支，也不会进入历史加载异常分支。

本次增量数据源修复已取消终态快照替换，收到的回复会留在数据源中。原生投影 warning 仍未转换成协议事件，手机无法据此显示“历史同步异常”；缓存淘汰或重连后的首次读取仍可能只拿到旧投影。不能将投影失败伪装成“发送失败”而引导重发。

## 原生错误通道实测确认

本机 codex-cli 0.153.4，使用受影响 thread 的数据库 backup 和 rollout 副本，独立 CODEX_HOME，未复制认证配置。模型端点指向本机临时 HTTP 服务，该服务固定返回诊断 400，未向真实模型发送请求。对副本执行 initialize、thread/resume、turn/start、thread/turns/list，同步捕获 stdout JSON-RPC 和 stderr；副本执行完关闭原生进程。测试脚本 `/tmp/codepet-resume-investigation/capture_warning.py`。

两次捕获结果一致：

- 四个 RPC 均返回 result，序号错误不在 response.error 中。
- stderr 捕获 18 条带 `expected ordinal 294, got 293` 的 JSON WARN 日志，target 为 `codex_thread_store::local::live_writer`。
- 全部 stdout 协议消息中没有该 ordinal 错误。两个 `warning` 通知分别是系统 Hook timeout 调整和诊断模型 metadata 缺失；唯一 `error` 通知来自本机端点预设的 400，表明捕获路径能收到原生 error，但序号错误没有通过此路径发送。
- 本次副本的内部 logs DB 未取得该错误，不能把 stderr 与 SQLite 日志写入在所有配置下视为同一保证。

第二次摘要位于 `/var/folders/_x/j99_pl_n56dgsr_ws034gfg80000gn/T/codepet-warning-capture-o_1tz69i/summary.json`。关键结论：原生通过 stderr 日志通道暴露此 warning，未通过 App Server JSON-RPC 错误或 warning/error 通知暴露它。Provider 的 `client/framing.rs::drain_stderr` 读取并 eprintln 转发，尚未转换成会话诊断事件，故手机不可见。该实测补齐了此前“stderr 是否输出尚未确认”的证据缺口；不等于原生序号错误已修复。

## 桌面端能继续执行并不表示历史投影已恢复

用户报告同一会话在 Codex Desktop 可以继续执行。再次只读核对：资源挂载 rollout 有北京时间 05:06:13 task_started 和 05:11:40 task_complete，历史投影仍停 ordinal 294 / 70 item；桌面进程 PID 2046 在 06:42:18 shutdown 时仍报同一序号错误。另一故障会话已有 14:10:55 started、14:11:15 complete，而历史投影仍为 ordinal 183 / 45 item，PID 2046 在 14:13:35 仍报错。这直接证明执行成功与投影失败可以同时成立，不能将“桌面端能执行”当作数据修复成功的证据。

已确认 CodePet 的发送也曾成功；故障时手机端依赖 conversation.get→thread/turns/list 读取旧投影，且当时的终态刷新可能覆盖已收实时内容，因而显示异常更明显。桌面 UI 在具体时刻采用内存事件还是其他历史读取路径尚未完整跟踪，不能仅凭现象断言它始终直接读取 JSONL。

修复需分层：CodePet 将确定的原生历史诊断关联到会话并显示同步异常，保护已经观察到的新 item 不被缺项快照覆盖；原生数据恢复则需在隔离副本上处理重复 ordinal 并验证历史投影重建，涉及 ordinal 的分叉边界与字节进度不能忽略。单纯清空 SQLite 投影仍会遇到原始文件的重复序号；尚未验证可安全应用到真实会话的恢复方案，未修改真实数据。


## 增量数据源与缓存修复（2026-09-06）

用户明确要求消息列表动态扩充：首次一页，手动上翻前插旧消息，实时事件在原位更新或尾部追加，任务结束不重新拉完整历史。已实现于 `codepet-remote`：

- `lib/gateway/gateway_client.dart`：get/resume 首屏只返回一页及 nextCursor；保留单页尺寸重试和路由验证。
- `lib/application/conversations/conversation_detail_controller.dart`：删除终态刷新；分页请求单飞，错误保留游标供重试，拒绝循环游标，历史页不更换实时事件窗口。
- `lib/core/domain/models.dart`：前插仅接受未见消息，保留实时内容和当前任务状态；稳定消息顺序防止前一轮输出移到下一轮用户消息之后。canonical item 原位替换 live item，终态停止流式显示。
- `lib/application/conversations/conversation_message_cache.dart` 与 `device_session.dart`：按访问顺序 LRU，默认 8 个会话、15 分钟闲置超时、每分钟清理；可见会话保留，后台事件不提升最近访问顺序。缓存中的窗口继续接收事件；淘汰关闭窗口，下次访问重载首屏。连接或 Provider 状态变化使相关缓存失效。
- `lib/features/conversations/conversation_detail_screen.dart`：更早消息入口实际触发单页网络请求，保留阅读锚点和分页错误重试入口。

验证：`flutter test --no-pub` 共 269 项通过；其中包含 390×844 手机尺寸分页锚点测试、离页事件与重新进入缓存测试、LRU/过期/运行时失效测试、分页期间实时消息保留与循环游标测试。最后补充 Provider 定向失效后，重新运行相关会话和设备测试，87 项通过；命令与结果见[规约](../60-rules/remote-conversation-message-sources.md)。未运行格式化工具，未构建或安装 APK。原生 JSONL/SQLite 未修改；这次修复不会恢复其已损坏的投影历史。
