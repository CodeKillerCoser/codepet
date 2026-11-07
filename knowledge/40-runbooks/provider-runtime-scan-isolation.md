# Provider 检测失败隔离与库存保留

## 现象

2026-11-08，用户确认另一台 Mac mini 在启动和重新检测时显示 `provider_plugin_unavailable: Provider plugin is not ready: dev.codepet.claude`，同时 OpenCode 已安装却不出现在手机入口。用户要求各 Provider 检测并行、互不影响。

## 证据

- 用户提供的 2026-11-07 电脑 `code-pet.log` 在 11:27:29 UTC 记录 Claude mux 握手超时；独立 OpenCode 事件中库存和 ready 早于手机连接，但手机握手仅收到 Codex。缺少握手当刻的 Host 库存及刷新取消时序，不能据此确定现场入口缺失的最终根因。
- 修复前 `src-tauri/src/lib.rs::refresh_agent_runtimes` 使用 JoinSet，在首个错误上以 `??` 返回。销毁 JoinSet 会取消其他检测，后续快照和更新事件也不执行。
- 修复前 `PluginManager::runtime_get_installed(refresh=true)` 在 RPC 前清空库存，响应不回填缓存，仅靠后续通知恢复。取消或失败与此组合，会使 Gateway 暂时或持续排除已安装的内置 Provider。
- 新增真实子进程测试确认：Claude 启动失败、Codex 探测超时期间，OpenCode 可以完成刷新；成功结果在慢项结束前已进入 Host 缓存，Gateway 保持可见。

## 根因

已确认的代码缺陷有两个：批量任务以任意单项失败作为整体失败并取消同批任务；缓存将“探测尚未成功”当成“安装清单为空”。前者扩大故障范围，后者把请求状态错误转换为产品可见性变化。它们是需要修复的机制，不等于已还原 Mac mini 现场全部时序。OpenCode 会话查询 HTTP 500 不由这两个机制解释。

## 引入历史

未确认具体引入提交。

## 修复方案

Host 先启动全部独立探测，再收集每项结果；单项错误、超时、任务异常不会取消其他探测。请求继续使用现有 RPC 超时。每项完成即可更新自身缓存及运行时变更通知；批量命令的最终返回仍等待各项结束。

保留上一次库存直至收到新的有效结果。刷新取消和 RPC 错误保留库存；扫描中的响应、带 scanError 且为空的结果保留之前的安装项及选择。干净完成的空结果正常清空，确保卸载后隐藏行为不被破坏。RPC 和通知共用安装逻辑，依据进程 generation、进程引用及请求 revision 防止旧响应覆盖新状态；扫描中转为完整库存仍可触发实例启动。

桌面批量入口直接从 catalog 选择内置 Provider，避免串行预读；列表读取也并行。每项错误保留在自身诊断中，健康项正常返回，不再把 Claude 的错误提升为整批命令失败。普通快照读取保留诊断，成功重试清除它。

## 涉及模块

- `crates/codepet-host/src/providers/manager/runtime_inventory.rs`：任务隔离、缓存和诊断一致性；manager 及 import 初始化新增状态。
- `src-tauri/src/runtime_gateway/tauri_bridge.rs`、`src-tauri/src/lib.rs`：批量命令、并行读取与逐项诊断展示数据。
- Host Gateway：生产过滤规则不变；通过检测期间的可见性断言验证缓存修复，补充测试快照字段。
- Host fake provider、manager_gateway 和桌面桥接测试：可控超时、取消、错误、通知及响应顺序，避免依赖本机真实安装情况。前端观察器和协议结构未改动。

## 验证结果

2026-11-08 本机运行：

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway --test builtin_provider_integration
TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo test --manifest-path src-tauri/Cargo.toml --lib runtime_gateway::tauri_bridge::tests
npx vitest run frontend/lib/providerRuntimes.test.ts frontend/lib/agentRuntime.test.ts
```

分别通过 43、6、11 项，共 60 项。Host 覆盖三个内置 Provider 的集成启动，以及 idle/restart 等已有行为。桌面裸 cargo test 先被已有 macOSPrivateApi 配置校验拦截；使用仓库既有测试配置覆盖后通过，未修改发布配置。`git diff --check` 通过。

## 回归防线

- 失败隔离及耗时风险：并发测试用门闩确保健康项在慢项仍待响应时完成；慢项走真实 RPC 超时。
- 取消与误隐藏：取消单次刷新、RPC 错误和扫描错误均保留 Gateway 入口；成功空扫描才隐藏。
- 陈旧结果及重复启动风险：测试通知先于 RPC、连续刷新和进程重启；三个内置集成用例验证扫描后实例可正常启动。
- 桌面错误扩大风险：桥接测试同时覆盖失败项、健康项、再次读取及成功恢复。前端既有测试检查快照更新和旧结果丢弃。

## 规约候选

已写入 [Provider 生命周期与界面派生快照](../60-rules/provider-dependent-ui-snapshots.md)，作为共享 Host/桌面检测入口的长期约束。

## 未知项

未在用户的 Mac mini 和手机重新进行 Release 实机验收，尚未构建、替换或发布版本；批量检测取消是否正好导致原始日志中的入口缺失仍需现场验证。OpenCode HTTP 500 的具体请求与服务端原因仍待诊断脚本成功获取响应。本次不修改其轮询、空闲策略或 VPN 通道处理。
