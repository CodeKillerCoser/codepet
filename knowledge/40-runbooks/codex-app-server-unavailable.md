# Codex Remote App Server 不可用

## 现象

- 远程 Provider 显示 unavailable/error，list/create/turn/approval 返回 `provider_unavailable` 或 timeout。
- `turn.start` 返回 `conversation_write_conflict`，但 `conversation.get` 仍可读取历史。
- Codex Desktop companion 和桌宠仍可工作。
- 日志出现 executable 解析、spawn、initialize、进程退出或 App Server request timeout。
- `conversation.get` 返回 `provider_response_too_large`；若仍出现 `App Server physical line exceeds 16777216 bytes`，运行的是保留旧原生行上限的 Provider，应核对实际二进制。

## 需要收集的证据

- `AgentRuntime` 的 Codex executable path、source、version 和诊断。
- Plugin Manager 的 Codex plugin/instance status、catalog diagnostic、process exit 与最近 stderr；不要记录 prompt、command、diff 或完整原生 payload。
- 区分 Provider 插件在线状态与 Harness 实例状态，记录唯一共享 Server 的 PID/generation、会话 route、请求 method/id；不记录业务正文。
- 历史读取只记录 Codex CLI 版本、`thread/read` 是否 metadata-only、turn page 序号、cursor 是否推进与帧字节数；不要记录 page payload、正文、command 或 diff。
- companion Provider 与桌宠是否仍保持独立状态，remote event 是否只存在于 remote replay。

## 排查步骤

1. 在运行时设置确认 Codex executable 已通过文件、执行权限和限时 `--version` 验证。无效手动路径不会静默回退。
2. 检查开发安装：`provider-plugins/codex/codepet-provider.json` 与相对 Provider executable 位于同一目录，`pluginId` 为 `dev.codepet.codex`，且没有重复 manifest。
3. 区分两层启动配置：Provider executable/环境和 `appServerArgs` 来自 manifest；`appServerExecutable` 必须只由 Agent Runtime resolver 以绝对路径注入。Provider 不补默认参数，也不搜索用户目录。
4. 检查 instance.start：每实例只 spawn 一个共享 Server，spawn 前登记占位，spawn 后、initialize 前登记 session；stop 应先回收 PID 再返回。记录 Host→Provider ping 的 sequence/client revision/连接数量，确认客户端集合已传入。
5. 对 turn 故障检查共享 Server 内的会话槽：首次 acquire 只在同一进程 resume(excludeTurns=true)，后续操作复用；cancel 与 resume 写入共用短 send gate，cancel 先发生时不得再写旧代。
6. 检查两层并发：普通 Host request 是 16 active + 32 pending，`instance.stop`/`instance.destroy`/`provider.shutdown` 是独立 2 active + 4 pending；队列满应按原 id 返回 retryable `provider_overloaded`，不得执行被拒请求。reader 不得因普通队列满停止读取，所以 16 个悬挂 write 后 stop、16+32 后 EOF 都应在 2 秒探针内完成回收。stdout 已断开时，response 或异步 event 的 write/flush failure 都必须只发送一次无输出依赖的 terminal 信号，不能只记日志后继续运行。
7. 检查释放点：页面退出、A 的终态均不关闭 Server；最后客户端离线或 Host 心跳超过 60 秒才关闭整个实例，包括 active turn。同一 clientId 的不同 socket 按 connectionId 分开计数。
8. 只在 `thread/resume` reject 同时满足 `code=-32600`、`data` 缺失/null 且 message 精确为 `thread <当前 conversation id> already has an active writer` 时，对外返回 retryable `conversation_write_conflict`，details 为 `operation=thread/resume` 与 `reason=owned-by-other-runtime`。wrong code、近似 message、其他 thread id 或非空 data 必须保留普通 Provider error；不得把原生 owner message 写入标准冲突错误或日志。
9. 若修改 executable 或点击刷新，确认 Host 更新同一个 Codex instance setting，并按 stop → start → manifest instance create/start 显式重启插件；replacement 的 RPC 必须反映新 setting，Tauri 内不应出现第二个 Server。
10. 对 timeout 或 process exit，不自动重放 create、turn、interrupt 或 approval。remote 结果不确定也不得触碰 Desktop companion；两条链路保持独立故障状态。
11. 单独确认 Desktop companion：其 socket、owner/revision 和 activity projection 不应因 remote 故障清空、重启或改用 Hook/transcript。
12. 对大历史单独分流：确认 Gateway/Provider `conversation.get` 请求携带 cursor/limit，Provider 只读取这一页并返回 pageInfo。若最终返回 `provider_response_too_large`，客户端必须保持原 cursor，将 limit 逐次减半后重试；成功后才使用 nextCursor。若 `limit=1` 仍失败，才判定为单 turn 过大。确认原请求 id、`details.maxFrameBytes=16777216`，并用后续 `project.list` 或其他原生查询及同 PID 证明 Harness 仍存活；不要增大统一 frame limit。

## 结果判断

- Provider manifest/binary 缺失：按 `../10-architecture/codex-provider-plugin-runtime.md` 的开发安装步骤恢复，再重启 Host。
- App Server executable 不存在或无效：修复/清除运行时配置后刷新 Codex Provider。
- initialize timeout：启动未就绪，回收该次启动；已 Ready 后单次 RPC timeout 返回 `provider_request_timeout`，共享 Server 保持在线，未知写入结果不重放。
- 共享 Server 退出：整个实例 Error，全部会话与审批 generation 失效；在线时后续 SDK 心跳会重试幂等 start，也可显式 refresh。
- 明确业务 RPC reject：只返回该请求错误，不关闭共享进程；未知发送结果不自动重发 turn/start。
- writer conflict：get 保持纯读；其他进程仍持有 writer 时返回明确冲突，不假定 unsubscribe 已立即释放。
- 单条协议响应不兼容：仅对应请求报错，更新 mapper/fixture 前不要放宽解析；坏消息 drain 后继续读下一帧。
- 原生历史响应超过 16 MiB：当前 JSONL reader 流式解码，不按大小拒绝。Provider 透传 cursor/limit 给 full turns API；只在生成 `kind: tool` item 时截断超大文本并记录 `_meta.truncations`。版本不再选择 item 分页。错误后以另一历史/项目请求及同 PID 验证 Harness 可用，不能只检查 provider.describe。
- 最终 Provider Frame V1 超过 16 MiB（header + raw/zstd payload）：当前请求返回 `provider_response_too_large`，Server 继续服务；Remote 使用相同 cursor 和更小 limit 重试，不推进 cursor。
- remote Provider 正常但手机仍不可访问：转查 Gateway LAN/WSS、配对、凭据和网络可达性；Server 是否停止由 SDK 的在线客户端集合和心跳期限决定。

## 恢复后验证

- remote Provider 从 unavailable 变为当前 replacement 的真实 ready 状态。
- `conversation.list/get/create`、turn start/steer/interrupt、approval 和通知通过真实 fixture App Server 子进程与 manifest-launched Provider binary 闭环。
- 重复 get 不创建进程或 resume；64 个会话复用同一 PID，A 终态后 B 继续工作；最后断开 PID 退出，Provider 仍可 describe，重连只启动一个新 PID。
- 两页以上历史按升序合并，跨页 item/content ID 稳定；最终投影超限返回 `provider_response_too_large` 后，`provider.describe` 仍成功。
- stdio 饱和探针中 16 个普通 write 全悬挂时 `instance.stop` 仍在 2 秒内返回；16 active + 32 pending 后第 49 个普通请求按 id 返回 `provider_overloaded` 且无 App Server 记录；随后 EOF 仍在 2 秒内退出并清零 PID。
- 延迟共享 Server initialize 连续五轮 start/stop，不得在 stopped 后出现 Ready；resume/cancel barrier 确认 stop 先发生时无 thread/resume/start 写入。
- 关闭 Provider stdout 后制造 Server 异步 fault，Provider 必须进入全局 shutdown；共享 Server PID 清零，不能依赖继续写 terminal event 才完成清理。
- terminal publication barrier 证明终态发布先于下个同会话操作；stop 后预取的旧 handle 不能向旧 generation 写入。
- fixture session 证据使用 PID 和 thread ID 共同定位；多会话共享同一进程，审批 request ID 在 Server 内必须唯一。
- remote conversation/turn/approval 只出现在 remote replay；companion replay 与桌宠 activity store 不变。
- Desktop companion 在整个 refresh/故障期间保持自己的 provider/session/sequence。
- 未启动 Hook、audit、transcript 或文件监听 fallback。

## 升级处理

- initialize 在有界 timeout 后仍留下无法终止的 Provider/App Server child。
- 插件 restart 后旧 process 继续发布事件，或审批被路由到新实例/其他 App Server session。
- Provider event 进入 companion replay/event 或桌宠 activity；这表示生产 wiring 发生跨链路污染。
- Codex CLI 升级改变 App Server request/response/notification wire 语义。
- limit=1 仍发生最终 Provider wire 超限：检查 item `_meta` 与字段类型；非 tool 巨大正文、众多中等字段或媒体不在当前截断范围。保留明确错误，先讨论新的内容策略，不能在 Provider 偷改 turn 数或添加整 turn 预算。
