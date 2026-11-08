# 默认空闲回收误停 Claude 与 OpenCode

> 历史修复记录：本文记录 ad5c0f6 的 SDK opt-in 过渡方案。2026-11-08 用户进一步要求迁至 Host，当前实现见 [Host release-control 调度](../10-architecture/host-provider-release-control.md)；下文的 idle_timeout / idle.rs 不再是现行接口。

## 现象

2026-11-08，用户补充另一台电脑上 Claude/OpenCode 可以启动，但过一会自动停止；同时明确空闲重启是 Codex 会话锁适配策略，其他 Provider 不应参与。

## 证据

- 已提供的 OpenCode 事件日志：2026-11-07 19:27:33.423（北京时间）Ready，19:32:34.360 Stopping，19:32:34.384 Stopped，间隔约 301 秒。
- 修复前 SDK `StdioServerOptions` 默认 `idle_timeout=300 秒`、`restart_after_idle=false`；mux 无条件创建 idle 定时任务。Claude/OpenCode main 直接使用默认值，因而空闲到期会停止且不会恢复。Codex main 另行设置 restart_after_idle=true。
- `runtime/idle.rs` 在 Ready、无活动且窗口到期时调用 instance.stop；该逻辑位于 Provider SDK，Host 没有统一下发定时释放请求。
- 用户此次没有提供新的 Claude 停止时间戳，不能把当前现象与之前 `provider_mux_error: transport handshake timed out` 当成同一个故障。前者有可复现的通用回收机制，后者发生在初始化前，仍需独立定位。

## 根因

SDK 默认开启了应由适配器选择的生命周期策略，导致 Codex 的空闲释放需求影响 Claude、OpenCode 以及使用默认配置的自定义 Provider。把所有适配器改为停止后重启也不符合用户要求。

## 引入历史

2026-11-05 的生命周期文档曾规定 SDK 默认 300 秒；未追溯具体引入提交。2026-11-08 按用户明确的适用范围收窄为 Codex 显式启用。

## 修复方案与模块

- SDK `runtime/options.rs`：`idle_timeout` 改为 `Option<Duration>`，默认 None；配置 Some 时仍拒绝零值。关闭不是“无限大的超时时间”，而是不启用策略。
- SDK `runtime/mux.rs`：仅 Some 时创建回收任务，None 不产生每秒唤醒；清理路径相应处理可选任务。`runtime/idle.rs` 保留原有活动保护、准入互斥与实例代际处理。
- Codex main：显式设置 Some(300 秒) 和 restart_after_idle=true；环境变量仍只覆盖 Codex 的正整数时长。
- Claude/OpenCode main 不添加回收或重启选项，使用 SDK 默认值即可保持就绪。其他 Provider 默认同样不参与。显式停止、shutdown、IPC EOF 的清理语义不变。
- SDK 分发说明：外部源码若显式填写旧 Duration，需要迁移为 Some(duration)；不需要回收的接入保留默认值。已经编译的旧插件需重新构建才能采用新 SDK 行为。

本次不增加 release-control 协议：当前调度不在 Host。未来若上移 Host，应通过协议能力声明筛选支持者，Provider 自身仍要校验活动状态；不支持的 Provider 不参与，不能退化成无条件 instance.stop。现有面向用户手动释放的 conversation.releaseInteraction 可以终止活动任务，不能直接作为自动空闲回收的替代品。

## 验证结果

```sh
cargo test --manifest-path sdk/rust/codepet-provider-sdk/Cargo.toml
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration -- --test-threads=1
npm exec --yes --package=bun -- bun test tools/cp-sdk-gen/cp-sdk-gen.test.mjs
```

分别通过 74、3、3 项，共 80 项；SDK 原有 5 个子进程 fixture 入口按原配置忽略。SDK 可控时钟模拟 8 小时空闲，默认策略不 stop/start，同一实例显式启用后才回收；覆盖 Some(0) 拒绝、活动任务、审批、等待输入、未知结果、显式停止不复活。Host 重新构建三个真实 Provider 二进制并配套原生 Harness fixture，验证三者初始化/连接/清理，以及 Codex 自动空闲重启和手动释放后的恢复。导出测试确认全部手写 SDK 模块与仓库一致。`git diff --check` 通过。

## 回归防线与规约

已更新心跳与任务保留规约、Runtime 生命周期、Claude/OpenCode 架构和 SDK 接入说明。评审任何默认生命周期变更时检查非 Codex 适配器及自定义 Provider；不能以“共用 SDK”为由统一业务释放策略。禁用回收后保持进程驻留是本次预期，退出清理仍由原有生命周期保障。

## 未知项

未在用户电脑安装 Release、实际等待 5 分钟后验收；当前时间边界来自可控时钟测试。Claude 启动握手超时和 OpenCode HTTP 500 的根因未由本修复解决。
