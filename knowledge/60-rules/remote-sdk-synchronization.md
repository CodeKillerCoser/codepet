# Remote SDK 同步与可空字段生成约束

## 证据与影响

2026-09-09 手机诊断日志记录 Codex、Claude、OpenCode 的 `provider.describe` 均因 `capabilities.usageDatasets: unknown field` 解码失败。桌面主工作区协议及生成 SDK 已包含该字段，而 Remote 主工作区签入的 Gateway SDK 不包含，能力加载失败会影响依赖它的最近、项目及详情入口。协议版本都为 v1 不能证明两端生成输出一致。

正式同步又暴露 Dart 生成器问题：可选且 schema 可空的 `UsageSummaries.peakDaily` 被生成成 `peakDaily!!`，Flutter 编译失败。修复应在 `tools/protocol-codegen/dart.mjs` 的可选字段序列化入口完成：已有非空条件时去掉外层 nullable 类型，再生成一次非空断言，随后重新生成 SDK。不得手改消费仓库的生成文件。

## 规约与验证

- 从桌面主工作区以 `cp-sdk-gen --package gateway --role client --lang dart` 和 `--package lan-channel --role models --lang dart` 同步 Remote 的两套输出及 lock 摘要；用相同参数加 `--check` 验证一致性。
- 桌面执行 `npm run protocol:check`，Remote 执行 Flutter 静态分析和 Gateway、会话及最近测试。仅检查生成文件新鲜度不能发现生成的 Dart 无法编译。
- 字段移除必须确认原业务是否仍需要。`06ffe4f` 在增加详情查询时误删了独立的 `runtime.usage` 概览；现已恢复 Provider 采集、协议字段、Gateway 脱敏转发和手机映射。不得用 SDK 同步掩盖该功能回归，也不得用详情查询替代概览。
- 生成器回归由 `tools/protocol-codegen/test.mjs` 检查 nullable 字段输出；Remote 的客户端测试使用包含 `usageDatasets` 的 Provider 描述，防止再次签入无法解析能力的 SDK。
- 手机已安装包必须重新构建安装，再验证真实设备的项目、最近和消息入口。源码及自动化检查通过不等价于真机验证完成。
