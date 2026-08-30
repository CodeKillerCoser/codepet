# Codex Remote App Server 不可用

## 现象

- 远程 Provider 显示 unavailable/error，list/create/turn/approval 返回 `provider_unavailable` 或 timeout。
- Codex Desktop companion 和桌宠仍可工作。
- 日志出现 executable 解析、spawn、initialize、进程退出或 App Server request timeout。

## 需要收集的证据

- `AgentRuntime` 的 Codex executable path、source、version 和诊断。
- Plugin Manager 的 Codex plugin/instance status、catalog diagnostic、process exit 与最近 stderr；不要记录 prompt、command、diff 或完整原生 payload。
- 子进程是否启动、stderr 有无有限诊断、请求 method/id 是否得到 response。
- companion Provider 与桌宠是否仍保持独立状态，remote event 是否只存在于 remote replay。

## 排查步骤

1. 在运行时设置确认 Codex executable 已通过文件、执行权限和限时 `--version` 验证。无效手动路径不会静默回退。
2. 检查开发安装：`provider-plugins/codex/codepet-provider.json` 与相对 Provider executable 位于同一目录，`pluginId` 为 `dev.codepet.codex`，且没有重复 manifest。
3. 区分两层启动配置：Provider executable/环境和 `appServerArgs` 来自 manifest；`appServerExecutable` 必须只由 Agent Runtime resolver 以绝对路径注入。Provider 不补默认参数，也不搜索用户目录。
4. 检查 Provider `instance.start`：它 spawn App Server 后执行 `initialize`/`initialized`，initialize 最多等待有界时间。silent child 应让实例转 error，不能阻塞 Tauri 或 Desktop companion。
5. 检查长期 reader/writer 和 request id map。乱序 response 应按 id 关联；无匹配 id、非法 App Server envelope 或 stdout/stderr 读取故障会使当前 session 走同一 terminal path，清空 pending 并终止子进程。超长物理行必须先完整 drain。
6. 若修改 executable 或点击刷新，确认 Host 更新同一个 Codex instance setting，并按 stop → start → manifest instance create/start 显式重启插件；replacement 的 RPC 必须反映新 setting，Tauri 内不应出现第二个 App Server 进程。
7. 对 timeout 或 process exit，不自动重放 create、turn、interrupt 或 approval。remote 结果不确定也不得触碰 Desktop companion；两条链路保持独立故障状态。
8. 单独确认 Desktop companion：其 socket、owner/revision 和 activity projection 不应因 remote 故障清空、重启或改用 Hook/transcript。

## 结果判断

- Provider manifest/binary 缺失：按 `../10-architecture/codex-provider-plugin-runtime.md` 的开发安装步骤恢复，再重启 Host。
- App Server executable 不存在或无效：修复/清除运行时配置后刷新 Codex Provider。
- initialize/RPC timeout：remote unavailable；保留 companion，检查 CLI 版本与 stderr。
- Provider/App Server 进程退出：remote unavailable；当前没有持续 supervisor，显式 refresh 或重启应用。
- 协议响应不兼容：remote error，更新 mapper/fixture 前不要放宽解析。
- remote 正常但手机仍不可访问：当前仓库尚无真实 WebSocket/P2P/relay transport，这是网络层未实现，不是 Provider 故障。

## 恢复后验证

- remote Provider 从 unavailable 变为当前 replacement 的真实 ready 状态。
- `conversation.list/get/create`、turn start/steer/interrupt、approval 和通知通过真实 fixture App Server 子进程与 manifest-launched Provider binary 闭环。
- remote conversation/turn/approval 只出现在 remote replay；companion replay 与桌宠 activity store 不变。
- Desktop companion 在整个 refresh/故障期间保持自己的 provider/session/sequence。
- 未启动 Hook、audit、transcript 或文件监听 fallback。

## 升级处理

- initialize 在有界 timeout 后仍留下无法终止的 Provider/App Server child。
- 插件 restart 后旧 process 继续发布事件，或审批被路由到新实例/其他 App Server session。
- Provider event 进入 companion replay/event 或桌宠 activity；这表示生产 wiring 发生跨链路污染。
- Codex CLI 升级改变 App Server request/response/notification wire 语义。
