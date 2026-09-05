# Provider 生命周期与界面派生快照

## 规则

依赖 Provider 的列表必须在其连接状态或 generation 变化后重新获取；单独更新连接标签不能视为数据恢复。正常初始化显示等待，真正停止或失败保留诊断。

## 适用场景

Host Runtime 安装列表、Remote 项目/对话列表，以及其他以 Provider 就绪为前提的数据。

## 反例

应用启动时并行请求得到 `provider_plugin_unavailable`，随后心跳显示在线，但 Runtime 列表仍永久保留第一次错误。或者旧请求晚返回，覆盖重连后的有效列表。

## 推荐做法

先注册监听，再读取初始快照；监听收到新状态后，忽略更早的快照回包。按 Provider ID、连接状态和 generation 判断依赖是否失效，合并进行中的重读，并丢弃旧请求结果。普通心跳及 Harness/turn 活动不能反复触发 executable 探测。退出组件时清理监听和未完成请求的回调。

## 来源

`../40-runbooks/host-provider-startup-stale-runtime.md`；Remote 的 Ready 后补拉见 `../40-runbooks/codex-detail-size-and-capability-loading.md`。

## 验证方式

使用延迟响应测试覆盖事件先于初始快照、初始化后在线、旧响应晚到、离线及新 generation、普通实例更新不重读和退出清理；页面验证等待文案、真实错误、操作可用性与焦点保留。
