# Remote LAN 产品稳定端口规约

## 规则

Code Pet 产品 runtime 必须优先绑定 LAN TCP `47622`，不能把 Host 重启后的重新上线完全依赖 mDNS。只有端口明确被占用时才允许告警并回退 ephemeral；Host 库默认与并行测试继续使用 port 0。

## 适用场景

适用于 Tauri `RemoteAccessRuntime` 的 listener 启动、retry、QR endpoint 和 mDNS 发布。`codepet-host::RemoteLanServerConfig::default` 不承载产品端口策略。

## 反例

每次启动都绑定 port 0 会让已保存的 WSS endpoint 失效；当 Android 模拟器、VPN 或系统 mDNS 未交付候选时，即使 Host 广播正常，旧配对也无法重新上线。把 47622 写进 Host 默认值又会让并行 listener 测试互相抢端口。

## 推荐做法

- 产品 runtime 显式传入 `0.0.0.0:47622`。
- 仅识别 `AddrInUse` 并记录稳定诊断后改用 `0.0.0.0:0`；其他错误 fail closed。
- QR、HTTPS/WSS URL 与 mDNS SRV 始终读取 listener handle 的实际端口。
- 不改变 LAN TLS identity、credential 或 Gateway wire。

## 来源

2026-08-31 安装新 Host 后端口由 60667 漂到 61658；Host mDNS 广播和 Android MulticastLock 均正常，但模拟器一分钟内没有收到候选，保存的旧 endpoint 无法恢复。

## 验证方式

Tauri runtime 测试分别验证首选端口成功绑定，以及人为占用后返回 available、实际端口回退且诊断为 `remote_lan_stable_port_unavailable`；Host listener 既有默认 port 0 测试保持不变。
