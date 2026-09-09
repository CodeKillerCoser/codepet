# 本地 Gateway 请求必须登记客户端在线状态

## 证据与原因

桌面用量选择 Codex 报 `No authenticated client is connected`，错误来自 `PluginManager::ensure_historical_route_ready`。Tauri 的 `codepet_gateway_request` 直接调用 `dispatch_for_caller_scope`，后者只绑定读者身份，不登记连接。启用连接心跳且没有 LAN 客户端时，请求在调用 Codex 前被拒绝，与 Codex 账号登录状态无关。原用量集成测试未启用心跳，漏掉了生产条件。

## 规则与修复

可信进程内通道通过 `dispatch_for_local_client` 登记请求期间的连接，复用连接快照及 Provider 心跳。RAII guard 在成功、失败或取消时释放；不能永久伪造远程设备在线，也不能删除 Manager 的无客户端检查。网络通道仍在认证后自行维护连接，不能调用本地准入入口。

Host Gateway 提供本地调度边界；Tauri 使用固定 desktop-main 身份接入；用量测试启用心跳，验证无连接调用被拒绝、本地查询成功及连接释放。请求登记只在调用期间有效，不表示窗口持续在线。

## 验证

`cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway codepet_usage_routes --quiet` 通过，覆盖实际 Provider 子进程、SQLite、空结果及本地准入。Tauri lib 编译检查验证命令调用边界。尚未通过真实 Codex 账户进行端到端查询验证。
