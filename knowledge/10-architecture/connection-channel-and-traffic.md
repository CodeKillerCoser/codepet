# 连接通道与应用流量展示

状态：2026-11-07 已实现并完成本地自动化与组件界面检查；尚未构建、安装本次正式 Release，也未以真实手机重做跨网验收。

## 背景与目标

设备在线标签不能说明连接是否使用局域网、IPv6 / IPv4 直连或中继，也不能反映实际流量。本次在桌面远程设备列表、Remote 首页及设备详情增加简短通道名、上传/下载速度、本次连接累计量。速度用于观察实际应用吞吐，非主动测速或链路容量估计；不暴露候选地址、端口、凭据等底层信息。

## 统计口径

- 两端均以本机为视角，↑ 为发送，↓ 为接收；发送与接收累计相加得到本次连接总量。重新建立通道从零开始，断线隐藏当前数值；不伪装成跨重连/跨进程的历史累计。
- 计数对象为传输适配器可见的 Gateway 负载，包括握手、心跳、RPC 和事件。WebRTC 协商 gzip 后计数 CPGZ 压缩负载；WSS 仍计数文本 UTF-8 负载，不包含底层 permessage-deflate 的压缩率。排除 WebSocket/CPG1/TLS/ICE 头、关闭控制帧和重传；两端同一消息的字节数可比较，不能用来核对运营商账单。
- 发送在传输 API 成功提交后计入，接收在解帧后计入。RTC 每个有效数据分片即累计，不能等大消息完整重组才跳一次；发送缓冲及网络在途数据使两端短时数字不完全一致。
- 使用单调时钟、100ms 桶与最近 3 秒窗口。速率 = 窗口内字节 / min(连接年龄, 3秒)，分母至少 100ms；UI 每秒读取。无新数据 3 秒后速度归零，累计值不减少。单位 B/KiB/MiB/GiB/TiB。

## 通道来源与职责

LAN 适配器直接标记局域网。RTC 使用实际选中的 ICE pair：任一侧为 relay 则中继；双方为 IPv6 则 IPv6 直连，双方 IPv4 则 IPv4 直连；信息不足显示识别中或不带地址族的直连。配置了 TURN、尝试过某条路径均不构成选中证明。现有 RTC stats 采样更新通道，不新增主动探测或测速请求。

Host `remote/traffic.rs` 管理计数与平滑；LAN/RTC 适配器记录字节；认证握手成功后 `RemoteConnections` 按连接及客户端归属保存计数器，连接释放同步移除；Tauri `list_clients` 附带活动连接快照。Desktop 仅在设备页每秒轮询，保留 in-flight 防重入；同一客户端多个活动连接分别显示。

Remote `ConnectionTrafficMeter` 为两种传输提供相同统计，endpoint resolver / route race 只转发胜出通道的快照；应用端通过可选 metrics port 读取，不依赖 Flutter、socket 或平台判断。`ConnectionTrafficView` 独立刷新并在 dispose 取消定时器，不让聊天/项目列表每秒整体重建。

## 同期 Provider 问题的证据与修复

修复前 SDK idle 只调用 instance.stop；Codex 未 opt-in restart，Remote 将 Stopped 视为不可操作。现通过 SDK 选项仅为 Codex 自动 stop→start，保留所有业务活动保护及手动释放语义，详见 [运行时生命周期](interaction-runtime-lifecycle.md)。

Remote 已直接消费 handshake.providers，残留内置项来自 Host 无条件投影插件实例。本次 Host 使用安装清单统一过滤握手/列表/心跳及状态事件，已安装但停止与自定义 Provider 保留，详见 [Provider 快照规约](../60-rules/provider-dependent-ui-snapshots.md)。引入历史未在本次追溯。

## 风险与验证

- 落败路径混入、跨连接污染：route race 测试只暴露胜出计数；Host presence 测试覆盖同手机多连接及断开移除。
- 只看到消息完成瞬时峰值：分片测试在收到部分数据时即断言累计量增长，并排除 CPG1 头与 Close。
- 通道误判：Rust/Dart 测试验证选中 pair、地址族、relay 优先、缺失信息；IPv6/IPv4 的实网仍需安装版验收。
- 数值溢出界面：Flutter 320px widget 测试、浏览器真实 Svelte 组件检查含 360px 内容区域、键盘聚焦操作按钮；Desktop 用容器查询避免窄内容区域按钮遮盖速率。
- 生命周期/可见性：SDK 全套、Codex idle/heartbeat binary、Host manager 与三内置 Provider 端到端 fixture；不停止用户真实任务。

## 本地检查

Host lib 87 项与 manager 原有 35 项通过；另补了握手/list/ping/事件过滤集成回归。内置 Provider 集成 3 项通过；SDK 72 项通过、5 项忽略；Codex idle 1 项、heartbeat 3 项通过。Remote gateway/discovery/session/home 合计 203 项通过，后补的实际 ICE 路径测试通过。前端流量/设备/bridge 11 项通过，Vite build 与 Tauri cargo check 通过。真实 loopback TLS/WSS + 原生 RTC 集成 3 项通过（另 2 项实网探针忽略），包含大消息双向计数和实际直连通道断言；Tauri remote_access 9 项通过。详细测试输出保留于本机 /tmp/codepet-*-tests.log 等文件，不将日志产物提交仓库。

全仓 Flutter analyze 仅报告原有 `lib/app/codepet_remote_app.dart:643` 缺少花括号的 info；本次修改范围无诊断。Host 并行 lib 测试中原有文件描述符总量测试受其他用例影响，串行全套通过。生产签名包安装、用户手机真实传输速度与系统实际网络流量差异尚未实测。
