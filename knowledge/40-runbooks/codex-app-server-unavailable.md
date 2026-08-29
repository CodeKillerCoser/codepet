# Codex Remote App Server 不可用

## 现象

- 远程 Provider 显示 unavailable/error，list/create/turn/approval 返回 `provider_unavailable` 或 timeout。
- Codex Desktop companion 和桌宠仍可工作。
- 日志出现 executable 解析、spawn、initialize、进程退出或 JSON-RPC timeout。

## 需要收集的证据

- `AgentRuntime` 的 Codex executable path、source、version 和诊断。
- remote Provider status、最近一次 initialize/refresh generation 和公开错误；不要记录 prompt、command、diff 或完整原生 payload。
- 子进程是否启动、stderr 有无有限诊断、请求 method/id 是否得到 response。
- companion Provider 与桌宠是否仍保持独立状态，remote event 是否只存在于 remote replay。

## 排查步骤

1. 在运行时设置确认 Codex executable 已通过文件、执行权限和限时 `--version` 验证。无效手动路径不会静默回退。
2. 确认启动参数是受控的 `app-server --listen stdio://`，没有监听公网，也没有使用 Desktop 私有 IPC 作为 remote fallback。
3. 检查 initialize：应用启动使用 unavailable placeholder 并在后台初始化，initialize 最多等待有界时间。silent child 应超时并转 unavailable，不能阻塞 Tauri 或 Desktop companion。
4. 检查长期 reader/writer 和 request id map。乱序 response 应按 id 关联；无匹配 id、非法 JSON-RPC 或 reader 退出会使 session fail closed。
5. 若修改了 executable 或点击刷新，确认 refresh generation 只安装最后一次 replacement；旧 adapter retire 后不得再发布事件，新 adapter activation 应发布其最新真实状态。
6. 对 timeout 或 process exit，不自动重放 create、turn、interrupt 或 approval。create 结果不确定时，Desktop 同期发现的新 thread 会保守排除，避免 remote task 进入桌宠。
7. 单独确认 Desktop companion：其 socket、owner/revision 和 activity projection 不应因 remote 故障清空、重启或改用 Hook/transcript。

## 结果判断

- executable 不存在或无效：修复/清除运行时配置后刷新 remote Provider。
- initialize/RPC timeout：remote unavailable；保留 companion，检查 CLI 版本与 stderr。
- 进程退出：remote unavailable；当前没有持续 supervisor，显式 refresh 或重启应用。
- 协议响应不兼容：remote error，更新 mapper/fixture 前不要放宽解析。
- remote 正常但手机仍不可访问：当前仓库尚无真实 WebSocket/P2P/relay transport，这是网络层未实现，不是 Provider 故障。

## 恢复后验证

- remote Provider 从 unavailable 变为当前 replacement 的真实 ready 状态。
- `conversation.list/get/create`、turn start/steer/interrupt、approval 和通知通过 fake peer/实机闭环。
- remote conversation/turn/approval 只出现在 remote replay；companion replay 与桌宠 activity store 不变。
- Desktop companion 在整个 refresh/故障期间保持自己的 provider/session/sequence。
- 未启动 Hook、audit、transcript 或文件监听 fallback。

## 升级处理

- initialize 在有界 timeout 后仍留下无法终止的 child。
- replacement status 倒序、旧 generation 继续发布，或并发 refresh 安装了较旧 adapter。
- remote task 通过 Desktop auto-load 进入桌宠，或 ambiguous create 后候选未被保守排除。
- Codex CLI 升级改变核心 JSON-RPC request/notification 语义。
