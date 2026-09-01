# Codex Remote App Server 不可用

## 现象

- 远程 Provider 显示 unavailable/error，list/create/turn/approval 返回 `provider_unavailable` 或 timeout。
- `turn.start` 返回 `conversation_write_conflict`，但 `conversation.get` 仍可读取历史。
- Codex Desktop companion 和桌宠仍可工作。
- 日志出现 executable 解析、spawn、initialize、进程退出或 App Server request timeout。

## 需要收集的证据

- `AgentRuntime` 的 Codex executable path、source、version 和诊断。
- Plugin Manager 的 Codex plugin/instance status、catalog diagnostic、process exit 与最近 stderr；不要记录 prompt、command、diff 或完整原生 payload。
- 区分 observer 与 conversation execution 子进程；记录有限的 generation/conversation route、请求 method/id、终态与 process exit，不记录 prompt 或内部 owner 身份。
- companion Provider 与桌宠是否仍保持独立状态，remote event 是否只存在于 remote replay。

## 排查步骤

1. 在运行时设置确认 Codex executable 已通过文件、执行权限和限时 `--version` 验证。无效手动路径不会静默回退。
2. 检查开发安装：`provider-plugins/codex/codepet-provider.json` 与相对 Provider executable 位于同一目录，`pluginId` 为 `dev.codepet.codex`，且没有重复 manifest。
3. 区分两层启动配置：Provider executable/环境和 `appServerArgs` 来自 manifest；`appServerExecutable` 必须只由 Agent Runtime resolver 以绝对路径注入。Provider 不补默认参数，也不搜索用户目录。
4. 检查 Provider `instance.start`：它只 spawn observer App Server，执行 `initialize`/`initialized` 与 `model/list`。observer 只应出现 `thread/list`/`thread/read`，不得出现 `thread/start`、`thread/resume` 或 turn/approval 写操作。silent child 应让实例转 error，不能阻塞 Tauri 或 Desktop companion。
5. 对 turn 故障检查 conversation execution：首次写应单独 spawn、在 initialize 前登记 Creating session、subscribe、`thread/resume`；同 active turn 的 steer/interrupt/approval 必须落在同 generation。cancel 与 resume frame 写入必须共用短线性化锁：cancel 先发生后不得出现 resume/start；resume 先写出后 stop 关闭 session，但不等待悬挂 response。
6. 检查两层并发：普通 Host request 是 16 active + 32 pending，`instance.stop`/`instance.destroy`/`provider.shutdown` 是独立 2 active + 4 pending；队列满应按原 id 返回 retryable `provider_overloaded`，不得执行被拒请求。reader 不得因普通队列满停止读取，所以 16 个悬挂 write 后 stop、16+32 后 EOF 都应在 2 秒探针内完成回收。
7. 检查释放点：running、waiting approval、waiting user input 与 Remote/WSS 断开不能释放；completed/failed/interrupted notification 必须先把槽切成 Closing，再发布 terminal event、关闭子进程、删除同 generation 槽并唤醒等待者。请求即使在 Closing 前预取了 Ready handle，也必须在拿 operation lock 后复核并改用新 generation，不能向旧 session 写。
8. 只在 `thread/resume` reject 同时满足 `code=-32600`、`data` 缺失/null 且 message 精确为 `thread <当前 conversation id> already has an active writer` 时，对外返回 retryable `conversation_write_conflict`，details 为 `operation=thread/resume` 与 `reason=owned-by-other-runtime`。wrong code、近似 message、其他 thread id 或非空 data 必须保留普通 Provider error；不得把原生 owner message 写入标准冲突错误或日志。
9. 若修改 executable 或点击刷新，确认 Host 更新同一个 Codex instance setting，并按 stop → start → manifest instance create/start 显式重启插件；replacement 的 RPC 必须反映新 setting，Tauri 内不应出现第二个 observer。
10. 对 timeout 或 process exit，不自动重放 create、turn、interrupt 或 approval。remote 结果不确定也不得触碰 Desktop companion；两条链路保持独立故障状态。
11. 单独确认 Desktop companion：其 socket、owner/revision 和 activity projection 不应因 remote 故障清空、重启或改用 Hook/transcript。

## 结果判断

- Provider manifest/binary 缺失：按 `../10-architecture/codex-provider-plugin-runtime.md` 的开发安装步骤恢复，再重启 Host。
- App Server executable 不存在或无效：修复/清除运行时配置后刷新 Codex Provider。
- initialize/RPC timeout：remote unavailable；保留 companion，检查 CLI 版本与 stderr。
- observer App Server 退出：Provider instance error；当前没有持续 supervisor，显式 refresh 或重启应用。
- 单个 execution 退出：owning conversation 的写操作失败并释放 writer；observer 与其他 active conversation 应保持可用。结果未知时由调用方基于原 client request id 决定后续，不在 Provider 内盲重试。
- writer conflict：保留纯读能力；确认真正 owner 终态退出后再由显式写请求创建新 execution。
- 协议响应不兼容：remote error，更新 mapper/fixture 前不要放宽解析。
- remote Provider 正常但手机仍不可访问：转查 Gateway LAN/WSS、配对、凭据和网络可达性；网络连接故障本身不应关闭 active execution。

## 恢复后验证

- remote Provider 从 unavailable 变为当前 replacement 的真实 ready 状态。
- `conversation.list/get/create`、turn start/steer/interrupt、approval 和通知通过真实 fixture App Server 子进程与 manifest-launched Provider binary 闭环。
- 重复 `conversation.get` 不创建 execution；active turn 操作复用同一 pid；terminal 后下一次写使用新 pid；释放 A 不改变 B。
- stdio 饱和探针中 16 个普通 write 全悬挂时 `instance.stop` 仍在 2 秒内返回；16 active + 32 pending 后第 49 个普通请求按 id 返回 `provider_overloaded` 且无 App Server 记录；随后 EOF 仍在 2 秒内退出并清零 PID。
- handle barrier 证明旧 Ready handle 在 terminal Closing 后不会写旧 session；resume/cancel barrier 证明 cancel 先线性化时没有 `thread/resume`/`turn/start` 记录。
- fixture session 证据来自每 PID 独立日志；运行 64 轮 terminal/approval 释放压力测试，不应出现 PID 记录丢失或交叉。
- remote conversation/turn/approval 只出现在 remote replay；companion replay 与桌宠 activity store 不变。
- Desktop companion 在整个 refresh/故障期间保持自己的 provider/session/sequence。
- 未启动 Hook、audit、transcript 或文件监听 fallback。

## 升级处理

- initialize 在有界 timeout 后仍留下无法终止的 Provider/App Server child。
- 插件 restart 后旧 process 继续发布事件，或审批被路由到新实例/其他 App Server session。
- Provider event 进入 companion replay/event 或桌宠 activity；这表示生产 wiring 发生跨链路污染。
- Codex CLI 升级改变 App Server request/response/notification wire 语义。
