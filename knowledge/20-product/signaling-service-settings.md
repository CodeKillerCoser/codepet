# 信令服务地址设置与自动设备登记

## 背景

2026-11-06，已安装 Mac 缺少公网配置，VPS 中也没有当前 Host ID；其他测试身份产生的 offer 不能证明该 Mac 可连接。用户要求地址由 App 维护，最简填写 IP 后自动登记，后续可以迁移 VPS。

## 目标

两端设置填写 IP、域名或 HTTPS URL 后立即登记；身份和凭据由 App 创建、持久化及恢复。修改服务不泄漏原服务凭据，不中断已有 LAN/RTC 会话。

## 非目标

Google OAuth 留到后续接入。设备登记不等于手机获得电脑访问权限；原有配对确认、签名校验、授权撤销继续生效。本次不承诺解决所有运营商 NAT 限制。

## 现状理解

旧版 `connections/rtc-cloud.json` 混合了服务地址、Host token、seed 和 peer 授权，需预先配置。旧配置留空选择时仍可运行；显式选择相同服务时使用原 token 登记，服务端需验证旧 bearer 才允许绑定公钥。

## 实现路径

无协议的地址默认 HTTPS/8443，完整 HTTPS URL 默认 443，支持 IPv6 与显式端口；拒绝 HTTP、凭据、查询、片段和业务路径。两端先保存公开地址和设备秘密，再发送签名登记请求。超时后复用同一身份重试，避免服务器已登记而客户端丢失响应时生成冲突身份。

`POST /v1/devices/register` 校验 Ed25519 签名；签名包含版本、操作、deviceId、role、tokenHash 和有效期。原始 JSON bytes 参与签名，seed/token 不上传。首次 Host 登记创建自己的信令作用域；Remote 登记仅建立安装身份，不授予任意 Host、TURN 或 RPC 访问权。同一 ID/角色/公钥/token hash 的重复登记幂等，替换身份返回 409。

Host 保留当前 DeviceRegistry ID，新增全局设备签名 seed 文件及按规范服务 URL 哈希命名的配置。不同服务共享设备身份，但分别生成 token、保存 peer；新文件原子写入且 Unix 权限为 600。Remote 使用已有 clientId，seed 与服务 token 存在安全存储，公开地址存在偏好设置。重启重新登记；Host 401 后重新登记，Remote 失败每 30 秒重试，成功后本进程缓存登记结果。

桌面保存时仅切换信令工作任务，LAN listener 与 SessionRegistry 保持原实例；手机后续建立连接时使用新设置，健康连接不被强制拆除。更换独立服务后双方选相同地址并重新连接/配对；迁移同一域名且保留服务数据库则保持授权。未来 OAuth 可作为登记接口的账号准入策略，客户端不绑定某台 VPS。

## 涉及模块

- Host `remote/signaling_service.rs`：统一规范地址；`cloud/registration.rs` 管理身份、服务凭据及登记；`cloud/service.rs` 控制运行时热切换。
- 桌面 `AppSettings.connection`、`configure_signaling_service` 与通用设置组件：公开地址独立持久化，专用命令保存后等待登记反馈，旧 settings caller 不清空新字段。
- Remote `ChannelPreferences`、`SignalingDeviceRegistration`、设置页及 App 生命周期：保存、自动登记、恢复及重试；配对和 routed transport 继续校验服务归属，禁止转发异域凭据。
- 服务端 `registration.py`：登记表、签名验证、冲突与准入边界；`server.py` 接入路由/审计；`deploy_registration.py` 增量部署、数据库备份和失败回滚。

## 风险与验证

- 重复登记、掉响应、重启及换服务：Host 持久化测试、Dart 注入网络测试与服务器幂等测试验证稳定身份、隔离 token。
- 身份抢占/权限扩大：服务器拒绝换 key/role/token，旧 Host 迁移必须证明 bearer；测试确认 Remote 登记后不能访问 Host/TURN。当前无账号系统，新设备自动准入是本轮明确选择；配对仍需确认。
- 流量和容量：请求体/签名长度有界、登记 30 次/分钟/后端来源、默认最多 1024 个登记设备。当前反代来源是 loopback，登记频率预算由此共享；未来多用户部署需配置可信代理来源策略和账号配额。
- 日志泄密：只记阶段、状态和设备关联，不记录 seed、token、登记信封或请求正文；服务器测试覆盖日志不含秘密。
- UI：窄屏保存、失败反馈和日志分享入口由 Flutter widget 测试覆盖；桌面组件焦点/布局预览已检查。

## 测试计划与当前结果

已运行 Host `cargo test --manifest-path crates/codepet-host/Cargo.toml --lib remote:: --quiet`，47 项通过；桌面定向设置测试 1 项通过。Remote `flutter test --no-pub test/settings test/gateway test/pairing test/app`，154 项通过；新增模块分析通过；Vite 构建通过。Python 本地 discovery 14 项、VPS server/registration 13 项通过。

2026-11-06 VPS 已增量部署自动登记，备份 `/opt/codepet-signal/backups/registration-20261106T125333Z`。全新 Host 仅设置服务 URL 的原生实测通过：重复保存保持 LAN 端口，自动登记、配对授权同步、真实 TURN relay、Gateway 握手、72 KB RPC 和撤销关闭。命令为设置 `CODEPET_SIGNAL_REGISTRATION_URL` 与唯一 `CODEPET_RTC_PROBE_HOST_ID` 后运行 ignored `rtc_public_relay_probe`，不预置 token 或 seed。

## 知识沉淀

更新公网部署及当前 Host 缺失配置的 runbook；身份规约补充登记与配对隔离、服务凭据绑定及先落盘后请求约束。

## 未知项

用户真实手机的移动网络配对/连接仍需新 Release 实测。电脑已构建并替换 Release，使用实际设置 UI 输入 `172.96.254.12` 后在 638 ms 内登记成功，重启后 819 ms 内恢复；VPS 登记表确认当前 Mac 的真实 Host ID 仅一条记录。Remote 使用真实 `SignalingDeviceRegistration` 的网络探针也完成首次及重复登记（仅安全存储使用测试替身，HTTP/签名/响应验证均为生产代码）。两种探针身份已从 VPS 清理。手机正式签名 Release 构建运行号为 42221630424，实际手机验收尚未执行。

## Release 交付验收

桌面代码提交 `42524e6`，本机 `/Applications/Code Pet.app` 已替换；Remote 代码提交 `ca9e4ca`，GitHub Release 构建 42221630424 成功。APK 为 `com.codepet.remote` 0.1.1/code 2，无 debuggable 标志，正式证书 SHA-256 与上一版一致（`cee49b08…e2665`），可覆盖安装。

[手机 Release APK](https://172.96.254.12:8443/downloads/codepet-remote/389886e1f079c8440ee2fdc7/codepet-remote-release-ca9e4ca.apk) 已从最终 URL 完整下载并校验，SHA-256 为 `ce330d276ec75951acaaabb6f6602c81a50171e43e8d16ee0088458c99e2825d`；同路径 `.zip` 返回 200 并支持 Range。nginx 配置测试通过，信令/coturn/nginx/xray 均 active。默认 LAN listener 集成测试另有 3 项通过（2 项需要授权的现场探针默认跳过，其中 relay 探针已单独运行）。用户手机的 4G 验收仍需安装此版本后执行，不能由这些结果推断所有网络均可穿透。
