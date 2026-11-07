# LAN 与 WebRTC 并行通道竞速

## 背景

用户要求可用通道包括 LAN、WebRTC IPv6 点对点、IPv4 点对点和 VPS 中继，并明确四者应并行探测、先通先用。旧 Remote 等 LAN 超时后才启动一个混合 ICE peer；旧信令服务按 host/client 保存单个 offer，新请求会覆盖旧请求；Host 串行回答 offer。仅修改候选优先级不能满足此要求。

## 目标

四路独立连接、首个可用通道获胜；只在赢家执行业务握手和 RPC，释放落败尝试。IPv6/IPv4/relay 策略必须在两端保持一致，不能把中继成功误报为直连。

## 非目标

不迁移已健康的连接，不自动重放业务请求，不改变配对身份和业务协议。不开通 VPS IPv6，不实现 TURN TCP/TLS 原生后端，也不改变既有 WebRTC 设置的启用状态。

## 现状理解

LAN 使用 pinned WSS、bearer 与发现解析；云端使用配对后的独立 token、公钥和签名 SDP。Gateway 通道打开后仍执行原业务握手。当前 VPS 只有 IPv4，但 IPv4 信令可交换双方 IPv6 候选，IPv6 直连不需要 VPS IPv6。

## 实现路径

开启 WebRTC 时，Remote 同时启动 LAN resolver 与公网授权加载；有授权即同时创建 IPv6、IPv4、中继三个 peer。LAN 独立预算 8 秒，RTC 各 55 秒，安全存储加载最多 10 秒；不累计为串行等待。无公网授权仍允许 LAN 获胜。

签名 offer 增加 `pathPolicy=ipv6-direct|ipv4-direct|relay`，Host 签名 answer 回显同值，Remote 校验与原 offer 相同。Host 对未知策略拒绝分配 peer；旧客户端无字段时保持原 auto 行为，新客户端云端路径需要新 Host。IPv6 不请求 ICE 服务，只保留 IPv6 非中继候选；IPv4 仅配置 STUN；relay 仅配置 TURN 且强制中继。Host 通过 SettingEngine 限定直连 peer 的 UDP 地址族，双方 SDP 过滤保持 fingerprint、ufrag 等身份信息。

信令按 host/client/attempt 保存独立信箱，保留 60 秒过期、全局 128 个上限，每客户端最多 12 个，允许竞速和短暂重连重叠。Remote 关闭时 best-effort DELETE 对应 attempt；Host 每次按可用名额领取最多 32 条 offer，JoinSet 并发处理，不因某个 ICE 收集超时阻塞其他请求。SessionRegistry 继续约束会话总量及撤销。

赢家确定后，取消落败尝试的事件订阅，关闭 native peer、信令和定时器。迟到成功不能替换赢家，落败错误不进入业务会话。LAN bootstrap 后台执行，不拖延 LAN 可用时间；仅中继赢家安排 TURN 凭据续期重连。每路独立验证身份，任何成功路径都不能绕过本身的证书/签名校验。全部失败时保留认证类不可重试错误。

## 涉及模块

- Host `remote/channels/webrtc/{mod,path_policy,cloud}.rs`：落实策略、签名绑定、并发协商及资源回收。
- `server/server.py`：避免同设备多路径 offer 相互覆盖，提供有界批量读取与取消。
- Remote `gateway/webrtc/{routed_rtc_transport,path_policy,cloud_signaling,webrtc_gateway_transport}.dart`：竞速、候选隔离和赢家生命周期。
- `remote_lan_listener.rs`：原生探针增加策略、随机测试 Host 身份与实际候选对校验。

## 风险

- 并发资源增加：信箱、Host 协商任务、SessionRegistry 均有界；回归覆盖超限、取消和撤销。
- 迟到回调覆盖赢家或泄漏错误：四路分别先成功、超时、关闭和迟到完成的单测验证。
- 配置 TURN 不代表使用 TURN：原生探针读取 nominated pair 的实际 candidateType/IP 地址族，验证握手、大消息和撤销。
- 老 Host 不回显策略：新 Remote 拒绝该云端应答；需同步升级两端，LAN 仍可独立成功。
- 运营商阻断 UDP、IPv6 防火墙或 NAT 行为：需要真机跨网验收，源码/同机探针不能替代。

## 测试计划与结果

2026-11-06：Host 全量 lib 76 项通过；`remote_lan_listener` 非 ignored 的 3 项通过；Tauri lib cargo check 通过。Remote Gateway/发现/App/架构/设置 152 项通过，相关 flutter analyze 无问题，Debug APK 构建通过。信令和部署备份 7 项测试在本机隔离 Python 环境及 VPS venv 均通过。

原生公网探针使用独立临时 Host 授权与正式 VPS 信令，在同一 Mac 上启动两个 Rust peer。IPv6、IPv4 直连与强制 TURN/UDP 中继均实际验证选中候选对、Gateway 握手、72 KiB 级请求/响应分片和凭据撤销；分别约 5.5、5.3、14.1 秒。中继断言双方候选均为 relay。信令服务并发版本已部署，外网 TLS health 返回 200。临时测试身份与私有配置已清理。此记录不表示手机跨网 NAT 穿透已验收。

复现命令：配置私有 `CODEPET_RTC_CLOUD_PROBE_CONFIG`，`CODEPET_RTC_PROBE_HOST_ID` 指定独立测试身份，`CODEPET_PROBE_PATH_POLICY` 分别设为 `ipv6-direct` / `ipv4-direct` / `relay`，运行：

```sh
CARGO_TARGET_DIR=src-tauri/target cargo test --manifest-path crates/Cargo.toml \
  -p codepet-host --test remote_lan_listener rtc_public_relay_probe -- --ignored --exact
```

使用独立测试身份，结束后删除 VPS 测试 Host/client 授权和两端私有配置，不能复用生产 Host token 启动 fixture。

## 知识沉淀

公网信令 runbook 与 Remote `docs/webrtc-channel.md` 同步更新。通用竞速隔离规则写入 `knowledge/60-rules/connection-race-isolation.md`。日志轮转问题另见 `knowledge/40-runbooks/logrotate-include-backups.md`。

## 未知项

手机 Wi-Fi/4G 跨网竞速、双侧不同 NAT、公网 IPv6 防火墙、多设备同时连接和 24 小时续期尚未实测。本轮没有替换桌面安装版或向手机安装 APK。关闭 WebRTC 时仍只使用 LAN/WSS。
