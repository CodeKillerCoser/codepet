# 多路径配对的授权与结果收敛

## 规则

同一邀请的多路径请求必须在 Host 授权层绑定一个逻辑申请，并只签发一次 credential。路径成功必须指完整身份校验后的有效成功结果，不能按 HTTP 最先返回决定。

## 适用场景

二维码邀请、LAN/VPS 首次配对、响应丢失重试和信令路径扩展。

## 反例

直接并发调用旧一次性 exchange 会使一条路径消费邀请，另一条报错；以 Future.any 选择会让最快的错误覆盖较慢成功。取消失败路径时撤销 credential 会误伤获胜路径。只验证 VPS TLS 无法证明结果由二维码对应的 Host 发出。

## 推荐做法

先持久化手机请求身份和密文，再由两路共享；Host 在同一锁内验证邀请绑定、查找已有申请或创建 pending。确认、拒绝和取消都遵守终态不可逆；接受后的同申请可取回同一结果，不同绑定拒绝。新邀请不得通过旧自动接受入口降级。加密结果绑定邀请、手机请求 hash 与 QR 公钥；业务层只提交一次 registration。路径关闭只清理自己的 I/O，不撤销共享授权。

后续 WebRTC 连接竞速必须另行解决 PeerConnection/session 所有权，不能从配对竞速直接推导成同时调用两套 connect；VPS 信令和 ICE relay-only 是两个独立选择。

部署验证必须核对运行版本实际使用的身份与配置目录。公网 token 绑定具体 Host ID；目录重构后发现旧配置，先比对身份，不能把旧 token 复制给另一个 Host。确认 v2 QR 已含公网元数据，再做无法使用 LAN 地址时的首次授权测试。

## 来源

`knowledge/20-product/dual-route-qr-pairing.md`；旧 QR 首次公网不可达的原因见 `knowledge/40-runbooks/webrtc-4g-qr-pairing-failure.md`。

## 验证方式

Host 的 invitation 状态机与真实 TLS 集成测试；信令的 scope/retry/immutable-result 测试；Remote 的 firstValid 和 encrypted-result binding 测试。变更协议后依 `remote-sdk-synchronization.md` 重新生成并检查两仓库 SDK，再验证新版 Android 与实际 Host。
