# Provider 项目错误被扩大为设备连接失败

## 现象与证据

2026-11-27 手机在连接电脑时显示 `Provider returned a project with invalid fields`。用户从钉钉与 chensi 的对话提供日志，下载文件 SHA-256 为 `5202251be7c74052b3257895edf276e90d6375cf1de8677b9179c7b14c364227`。日志 codepet.log 的 1036–1045 行显示 relay 已能交换 RPC，project.list 返回 isError=true，随后 ProtocolGatewayClient.listProjects → DeviceSession._connect 报连接失败。直连网络另有超时，但该业务错误发生在已经可用的 relay 上。

独立 agy Provider 0.2.2 修正无根项目后用户确认恢复；这只消除了触发数据，未解决连接/业务错误边界。当前 Host `providers/manager.rs::validate_project_routes` 明确拒绝空 roots，允许零时间戳；不能继续把零时间戳写成此次报错的已确认根因。

## 根因与引入历史

Remote 原 `_connect` 在握手后 Future.wait 全部 Provider 描述，再 await 选中项的项目和对话，最后才设 online。任一步失败进入连接 catch，关闭事件窗和 client、清空握手及缓存并设 failed。等待屏障还使单项慢描述阻塞健康项可用。普通分页使用跨 Provider 的共享 loading/error，切换 tab 后仍受旧请求影响。引入提交未确认。

## 修复方案与涉及模块

- Remote `application/sessions/device_session.dart`：握手及事件订阅成功即可 online；连接失败处理仅承担会话/通道的实际失败，业务启动独立进行。
- `device_session_provider_loading.dart`、`provider_data_state.dart`：按 Provider/业务域记录状态，独立能力读取、初始列表、有限只读超时与局部重试；保留已加载数据，防止把能力未知当成不支持。
- `device_session_pagination.dart`：按 Provider/查询范围管理门、错误和游标，A 的挂起不挡住 B；临时事件窗重放快照之后的事件。
- `device_session_runtime_lease.dart`、`device_session_projection.dart`：按租约与投影职责拆分原长文件，保留对外 API 和单一状态所有者。
- `features/home/provider_data_notice.dart`、首页：在选中 Provider 下显示局部加载/失败和重试，真实设备连接提示保持独立；等待能力时不宣称不支持最近对话。
- Remote 相邻测试：连接完成与业务完成分开等待；按职责拆分既有长测试支持/连接用例，保留旧回归。
- Host fake Provider 与 manager_gateway 隔离测试：以独立真实子进程注入无效项目、错误、挂起和退出，验证 Gateway ping/handshake 及健康插件查询与发送。Host 生产调度已有独立进程/请求与缓存清单，本轮无需修改生产代码；为源码尺寸限制，将既有测试按生命周期、路由校验和历史恢复拆分。

请求以设备 generation、Provider epoch、查询范围和独立门 token 校验。Provider 同 ID 同 generation 被移除再加入也不能接收旧回包。连接先上线后，各列表仍在 RPC 前打开事件窗，按响应 snapshotCursor 合并后续事件，保留全局事件订阅与真实断线检测。不自动重放写入或已受理 turn。

## 验证结果与风险

以下为提交前的本地验证记录，基线为 Remote `b1e6844907ff1ed37f1b382ad3f4c33c4fa9b265` 与 Host 工作树 `7a6d434e2add0daed90974755f592d2474ff86d0` 的工作区改动。正式构建与提交记录见下节。

- Remote `flutter test --no-pub --reporter expanded`：529 项通过；`flutter analyze --no-pub` 无问题。新增隔离、执行失败后健康发送、超时后晚回包、卸载重装、快照/事件以及 320/1000px 键盘重试用例，另覆盖能力未知不误报不支持、成功事件读取清除局部错误。
- Host `cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway`：59 项通过，含四种跨插件故障以及健康 Provider 发送。
- `npm run check:source-size`：通过；Remote session、独立 parts 和测试也均低于源码尺寸上限。`git diff --check` 检查两个工作区；保留原有用户改动。
- 只读超时不等于取消远端执行；迟到读取不回写，写入不自动重试。通道真正断开、认证拒绝、事件流错误与重连既有测试继续覆盖。
- 新行为在手机客户端；电脑当前安装的 Provider 修复不能替代安装新版手机客户端。正式 APK 需要既有发布签名，不能用公开调试密钥替代正式签名。本地调试 APK 与真机正式安装分别记录。
- Android arm64 Debug APK `0.1.3+4` 构建通过，buildRevision 为 `b1e6844-provider-isolation-20261127`，SHA-256 为 `eb280feb497680b8f3a61742795093d8bb625a57307613b5966672de750ab03a`。与 QA 模拟器原包的固定 Debug 证书相同，使用 `adb install -r` 保留数据覆盖安装，包元数据确认版本；没有用 adb 操作界面，也没有完成实体手机签名安装。该本地验证阶段未配置正式签名；正式构建由 GitHub Actions 的既有签名流程执行。

本轮双仓库的提交前补丁快照、测试日志、安装元数据和 APK 保存在 `artifacts/remote-provider-isolation-20261127/`；补丁快照用于追溯本地验证，不代表后续主分支的完整状态。

## 提交与正式构建

2026-11-27，Remote 修复提交 `9b26742eacb5425099ae3f3e45660f0eb1a37391` 已推送 `codepet-remote/main`。Host 测试与规约在最新主分支基线 `9dbc7b12` 上整合为 `a22ec701bdfb8e98fafcd606c4569820c4a634b3` 并推送 `codepet/main`；整合后的 59 项 manager_gateway 测试、源码尺寸检查通过，远端 Provider protocol conformance 与 Source size 也通过。

[Android Release 构建 46021601198](https://github.com/CodeKillerCoser/codepet-remote/actions/runs/46021601198) 成功，构建 headSha 与 Remote 修复提交完全一致。使用仓库既有签名 secrets，下载后的 APK 经 `apksigner verify --print-certs` 和 `aapt dump badging` 核对：

- 包名 `com.codepet.remote`，版本 `0.1.3+4`，Release APK 114363130 字节。
- APK SHA-256：`4fe3344aa2b0ec475893d6aa706be5d2617e6735ca2fc41d355cbfe40dfa0368`。
- 正式证书 SHA-256：`cee49b08b2bf2e20b1b862c1953782e4709615420dd4c7b5b2e5ab9e040e2665`，与本地 Debug 证书不同。
- [构建产物](https://github.com/CodeKillerCoser/codepet-remote/actions/runs/46021601198/artifacts/15207954646) 保留至 2026-12-11；本地副本与核验元数据保存在上述 artifacts 目录的 `codepet-remote-0.1.3-4-9b26742-release.apk` 与 `release-apk.json`。

正式包尚未安装至实体手机；手机网络、后台挂起和签名升级兼容性仍按下节验收。

## 回归防线与未知项

长期规则见 [连接与业务隔离](../60-rules/remote-connection-business-isolation.md)。新实现没有更改 Wire Schema、认证或 Host 的 Provider 数据校验。手机全链路签名安装、不同网络和后台挂起仍需使用发布包验收，不能以单元/Widget 测试冒充实体手机结果。
