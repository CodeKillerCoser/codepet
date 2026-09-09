# WebRTC 通道增量接入：实施与验收

## 背景

2026-09-09 用户启动实现，并明确三个范围约束：代码集中在通道层；保留现有 LAN 方式；暂不考虑文件挂载和预览。本文是本轮交付依据，覆盖此前连接提案中“删除业务 WSS”“所有 LAN 改用 RTC”“文件通道同步交付”的安排。

状态：**M0–M2 已在隔离工作树实现并验证：Windows Host ↔ Android 原生 RTC 的握手、大消息、心跳、事件与撤销通过。尚未合并；M3 公网信令/TURN、M4 公网与网络迁移验收未实施。默认仍为 LAN/WSS。** 用户随后指定创建并入库固定 Debug key；Debug APK 已构建且证书核验一致，Release 仍要求外部正式签名。

## 目标

- 新增 JSON-RPC over WebRTC DataChannel，复用现有 Gateway 请求、响应、事件、身份及任务生命周期。
- LAN 的 HTTPS 配对、发现、固定端口、证书 pin 和 WSS 保持可用，默认入口及已有设备记录兼容。
- 已配对设备可通过公网信令完成 RTC 建链，ICE 优先直连、需要时 TURN 中继；在中国大陆实际移动网络验证。
- 每个阶段独立审阅和回归，先证明抽取不改变 LAN，再加入 RTC；不以“建立 DataChannel”代替完整业务验收。

## 非目标

文件上传下载、目录挂载、资源 URI、网页预览、WebView、dev-server/HMR、文件专用通道不进入本轮。不增加音视频、摄像头/麦克风权限。Provider 协议、会话界面、业务 DTO 不随网络通道变化。公网首次配对另立范围，本轮先沿用可信 LAN 配对。

## 现状理解

检查基线：Host `v0@09e2fe8`；Remote `main@f88d4c9`。分别在独立 `codex/webrtc-channel` 工作树工作，主工作区未修改。

- Host `crates/codepet-host/src/remote/channels/lan/listener.rs` 将 WSS 帧与握手、ping、订阅、并发请求、trace、presence、credential 撤销放在一起。直接复制此实现会形成两套会话语义。
- `ProviderGatewayService` 已是业务分发入口；保留 caller scope、游标和断线后的任务行为。
- Remote `lib/gateway/transport.dart` 已定义 `connect/request/events/close`。`lib/discovery/resolving_gateway_transport.dart` 可注入 transportFactory，但它依赖 WSS URL 和 LAN 发现，不能直接当公网 RTC resolver。
- `lib/app/codepet_remote_app.dart` 是适配器组合入口。已有 `DeviceSession` 负责重连与 generation fence，不另写业务重连器。
- Gateway manifest 的 framing 仍写 `websocket-text`。新增 RTC 应补充通道描述与生成器验证；不可为了 RTC 改写方法或静默删除 WSS 声明。方法集合以实施基线 manifest 为准，不沿用旧提案的数量。

## 实现路径

| 阶段 | 交付及依赖 | 出口条件 | 当前状态 |
| --- | --- | --- | --- |
| M0 范围和边界 | 更新两个仓库的范围说明；记录依赖与验收 | LAN 保留、文件暂缓没有歧义 | 完成 |
| M1 共享会话层 | 从 LAN listener 抽出 channel-neutral session；LAN 只适配帧；依赖 M0 | Host 编译、现有 LAN 单测及集成测试通过，鉴权与生命周期不变 | 工作树实现与验证完成，待合并 |
| M2 RTC 互通 | Rust/Flutter DataChannel adapter、消息边界、发送背压；先用可信本地信令验证；依赖 M1 | Android↔Windows 真实 peer 上握手、请求、事件、超时、关闭通过；大消息有界 | 实现及原生互通通过；待合并 |
| M3 公网连接 | HTTPS 信令服务与两端 adapter、准入、TURN 配置、连接模式组合；依赖 M2 | 已配对设备离开 LAN 后能鉴权建链；强制 TURN 测试；LAN 默认仍可用 | 未开始 |
| M4 回归交付 | 双通道业务契约矩阵、大陆网络/真机验证、运维说明；依赖 M3 | 以下验收矩阵全部有结果，剩余限制明确 | 未开始 |

阶段入口由上一阶段验证决定，不按未经测量的日期承诺上线。开发角色为 Host 通道、Remote 通道、信令服务、验证；这是工作划分，当前没有创建独立代理任务或指派外部人员。

### M1：先收敛共用边界

新增 `remote/channels/session.rs`，接收已通过准入的 credential、共享 session registry、收发通道与原 Gateway。完整 JSON 消息是通道边界。帧编码、WebSocket ping/pong、RTC 分片与缓冲归 adapter；JSON-RPC handshake、协议 ping、事件游标、并发限制、trace 与退出清理共用。

草稿将已有逻辑移动而非重写：保留 64 项收发/请求队列、每会话 8 个并发请求、15 秒发送上限、2 秒入队上限、500 毫秒清理上限；现有 WSS 256 KiB 输入限制不改。准入仍在 LAN upgrade 前完成两次 bearer 校验，注册与校验次序保留。共享注册表是以后统一撤销两个通道的前提。

### M2：RTC adapter

已锁定 Rust `webrtc = 0.14.0`、Flutter `flutter_webrtc = 1.6.2+hotfix.1`，完成 Windows 编译及 Android 原生互通。只使用可靠、有序 DataChannel，显式关闭 Flutter 默认的 OfferToReceiveAudio/Video。首版只承载 Gateway JSON，无文件流，不引入 yamux。

RTC adapter 需要自己实现发送队列高低水位和完整消息上限；SCTP 的可靠传输不等于应用无需背压。根据两端协商的最大消息大小限制每片大小，拼装超限、超时、格式错误立即失败并释放缓存。不得一次将无限事件堆入 native send buffer。如果证明原生 DataChannel 无法满足约束，再专项比较 yamux，不能同时落地两套方案。

当前实现的精确契约见 `protocol/gateway/v1/webrtc-cpg1.md`：12 字节 CPG1 头、16 KiB 原生帧、请求 256 KiB/响应 4 MiB 上限、固定 5 秒拼装期限、64/16 KiB native 发送水位。旧长线提案中的文件分流与其他帧设计不适用于本阶段。

Remote 的 App 设置“开启 WebRTC”保存下次启动的通道选择，默认保持原 LAN/WSS。完全退出重开后生效，设置页显示当前通道；原 CODEPET_WEBRTC 编译期开关已由此设置替代。配对、preferredEndpoint、业务方法、SDK DTO 不变；Gateway manifest 仅增加 `additionalFramings`，生成器验证此元数据，生成源码无变化。

先使用已配对、证书 pin 的 LAN HTTPS 交换 SDP，可降低互通验证变量；信令必须是可替换接口。此验证依赖 LAN 信令可达，**不能标为公网连接完成**。信令代码不得进入 Gateway 请求 handler。

### M3：信令、准入和 LAN 共存

复用原连接提案中信令 rendezvous 与鉴权设计，独立实现 HTTPS 信令服务；SDP/ICE 有 attemptId、过期时间、大小限制和重放防护。Host/Remote 各自主动访问服务，不要求暴露 Host 入站端口。云端 deviceId 只负责路由，不代表可信身份；必须绑定配对身份、对端 DTLS 指纹和建链 attempt。Host 只保存 bearer hash 的现状必须在准入设计中处理，不能假设能从 hash 还原密钥。

TURN 凭据短期签发，密钥不进入 app 或日志。直连时业务不经信令；选中 relay 时业务持续经过 TURN。信令暂时不可用不得主动切断健康的数据通道，但影响新连接、重连及需重新协商的网络迁移。

保留独立 LAN resolver；新增 RTC 连接配置与 adapter，由组合入口选择，默认仍为 LAN。允许明确选择 RTC 进行验收；本轮不自动将所有旧设备记录迁移为 RTC、不删除 WSS。不要把 TURN/信令 URL 覆盖进已有设备的 LAN preferredEndpoint。RTC 失败后可重新建立 LAN 会话，但不得自动重发结果未知的非幂等写请求；沿用已有 outcomeUnknown 语义。

### M4：发布条件

先验证现有美国加州 VPS，不先采购新服务。部署信令/coturn 与修改系统配置是独立可审阅交付，当前尚未执行。实测 UDP、TURN/TCP、TURN/TLS，记录大陆 Wi-Fi、移动数据、运营商、直连/relay、建链耗时和失败原因；失败不能仅凭 IP 归因为带宽或地域。

最小可发布场景为已在 LAN 配对的 Android Remote 与在线 Windows Host。其他平台的编译/打包结果单列，不把未验证平台写成支持。连接诊断仅放通道基础设施，避免业务层到处判断 RTC/LAN。

## 涉及模块

| 范围 | 允许修改的原因与边界 |
| --- | --- |
| Host `remote/channels/` | 会话抽取、RTC framing、ICE、信令客户端及清理的主要落点 |
| Host `remote/access.rs`、runtime 组合入口 | 仅在跨通道准入/撤销和启动 RTC 所必需时调整；不重写配对业务 |
| Remote `lib/gateway/`、新增 RTC adapter、`lib/app/` | 实现原 transport 契约、选择通道；domain/features 不认识 SDP、TURN、native peer |
| Cargo/pubspec 与平台配置 | 引入/锁定数据通道依赖及必要网络设置 |
| 通道协议描述、生成器校验与两端测试 | 描述支持的 framing；业务 schema 和方法签名保持一致 |
| 独立信令服务与部署文档 | 公网建链所必需；不存储或转发业务 JSON-RPC |
| 既有 Gateway、Provider、界面及资源模块 | 不属于本轮功能开发范围；若必须触及，先记录具体原因和回归覆盖 |

## 风险与测试计划

| 风险 | 验证方式 / 必须通过的结果 |
| --- | --- |
| 抽取破坏 LAN 身份、订阅、关闭 | 运行原 `remote_lan_listener` TLS/WSS 集成测试和 registry 单测 |
| 两种通道业务分叉 | 同一组握手、请求、错误、事件 fixtures 分别通过 LAN/RTC；不增加按方法名分支 |
| 大消息/慢接收方耗尽内存 | 超过 64 KiB 消息、上限边界、恶意总长、缺片/关闭、native buffer 高水位测试；检查有界队列 |
| 业务请求阻塞 ping | 慢请求、事件洪峰时 heartbeat 仍成功；并发/超时上限保持 |
| 双通道撤销不完整 | 同凭据同时连接 LAN/RTC 后撤销，全部退出且 presence 清零；另一凭据保持 |
| 重连重复写入/旧事件污染 | 非幂等请求途中断线不重发；generation fence、cursor replay 与 pending cleanup 回归 |
| 信令冒充/泄漏/重放 | 错误凭据、错 DTLS 指纹、过期 attempt、重复 answer、跨设备投递均拒绝；日志脱敏 |
| TURN 没有实际被使用 | 强制 relay 并确认选中的 candidate pair；断开信令后健康通道继续传输 |
| 新模式破坏已有设备 | 未配置 RTC、旧设备、信令不可达时 LAN 配对/发现/WSS/忘记设备测试 |
| 大陆网络、手机后台恢复 | 真机 Wi-Fi↔移动网络切换、前后台、Host 重启；记录实际恢复与失败行为 |

建议执行命令（未运行不能标通过）：

- Host：`cargo check --manifest-path crates/Cargo.toml -p codepet-host`。
- Host：`cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib`。
- Host：`cargo test --manifest-path crates/Cargo.toml -p codepet-host --test remote_lan_listener`。
- Remote：`flutter test test/gateway test/discovery test/devices test/architecture test/app`；接入 native 后另跑 Android 构建与真实 peer 集成测试。
- 协议描述变更时执行仓库 codegen 校验并审阅生成差异，禁止夹带业务变更。

### 本轮实际验证

- 两个主工作区起始均 clean，独立工作树建立完成。
- M1 Rust 文件通过 rustfmt 检查；`git diff --check` 通过。
- 首次 cargo check 被 Windows 应用程序控制策略以 `os error 4551` 拦截；2026-09-09 按用户要求重试后未重现，未修改系统安全策略。重试发现并修正 LAN 撤销路径缺少共享 `CONNECTION_CLOSE_TIMEOUT` 导入的问题，随后 `cargo check --manifest-path crates/Cargo.toml -p codepet-host` 通过。
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib --test remote_lan_listener`：58 个单测与 1 个 LAN 集成测试全部通过。仍有原有 `RecentSnapshots::page` 未使用警告。
- LAN 测试初次在超大帧发送的 unwrap 处出现 Windows 10054。独立、未修改基线 `09e2fe8` 复现同一失败。测试现接受服务端提前关闭引起的限定发送错误，继续验证连接关闭与会话释放；没有改变生产代码的超大帧拒绝行为。详见 [断开类网络测试](../60-rules/network-rejection-test-timing.md)。
- M2 Host 验证：61 个单测、2 个 LAN/RTC 集成测试通过；手动 Android fixture 默认 ignored，专项运行通过。`npm run protocol:check` 21 项通过，生成文件保持最新。
- M2 Remote 验证：`flutter test test/gateway test/discovery test/devices test/architecture test/app` 180 项通过；随后新增 TLS 不重试防线并针对 RTC/配对运行 14 项通过，最终 RTC 8 项通过；新增 adapter、TLS 与原生探针静态分析通过。
- Android RMX3366 与 Windows 同 Wi-Fi 真机：实际构建安装探针后，纯 DataChannel 握手、72 KiB 级 UTF-8 请求 ID 的双向分片、protocol.ping、event.subscribe 后通知、HTTPS 撤销使 RTC 断连均通过。HTTPS 使用 ADB reverse，RTC 未配置 TURN，故此结果只证明本地原生互通，不证明公网能力。
- 原生构建遇到 Kotlin 跨 C/D 盘增量缓存失败；本次临时禁用增量并使用 in-process 编译后成功，Gradle 原配置已恢复。纯数据协商校验还发现并修正 Flutter 默认声明接收音视频的问题；原生重测通过。
- 按用户最新要求，Remote 创建并纳入固定 android/keystore/codepet-debug.keystore，本地和 CI Debug 共用；Release 保持 CODEPET_RELEASE_* 外部配置。flutter build apk --debug --dart-define=CODEPET_WEBRTC=true 成功，apksigner 验证包名 com.codepet.remote、证书 SHA-256 为 15d8c13ab1ae0aefa0010c7a90d7f2eed2883be2f1be9ec399beb7555d537860。assembleRelease --dry-run 在缺少正式配置时按预期拒绝。既有不同签名安装包的覆盖未验证。
- 临时 Host 探针已停止并删除凭据配置，三个本次 ADB 转发已移除。测试包已不在设备包列表中；额外卸载返回未能删除，因此清理结论以查询为空为准。
- 实现仍留在隔离工作树，未合入主工作区、未推送。下一阶段是 M3 公网信令/TURN。复现步骤见 [原生探针 runbook](../40-runbooks/remote-webrtc-native-probe.md)。

## 知识沉淀

本页跟踪本轮交付；旧连接提案保留长期设计但顶部注明本轮范围覆盖；Remote 文件方案标记暂缓。实现后用实测版本、互通结果及部署 runbook 更新本文，不将目标描述成已实现能力。

## 未知项

- 首次 Windows 4551 拦截原因未确认；重试已成功，当前不再构成阻塞。
- Release 正式签名 key 的本地位置/加载配置；不影响仓库固定 Debug key 构建。
- TURN/TLS、公网信令、网络迁移、其他平台的实际互通；本轮仅验证 Windows↔Android 的本地链路。
- VPS 域名/TLS 配置、coturn 端口与流量预算的实际部署状态及大陆网络质量。
- 公网信令的最小存储/清理策略及凭据升级设计，要在 M3 开发前固定并测试。
