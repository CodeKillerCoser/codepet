# 桌面 App 与 Android App 的协议联合测试

## 范围与前置条件

验证当前 `protocol/v2/` 生成的两端 SDK、独立 Provider stdio、桌面 App 内的实际 Host，以及 Android 原生 LAN/WSS、WebRTC 和生产 UI。自动化 Host 服务探针、单独 SDK 编译、Widget mock 均不能替代这条联合测试。

CodePet 与 Remote 工作区分别为本任务工作区及 `/Users/wangxin/Documents/codepet-remote`；以各自 git status 为准，保留已有改动。Remote 使用仓库固定 debug keystore，不修改签名身份。无实体手机时可使用 API 36 ARM64 模拟器，但不能声明覆盖真机、蜂窝或公网 TURN。

## 生成与构建

从 CodePet 的当前协议工作区执行：

```sh
npm run protocol:generate
npm run protocol:check
npm exec --yes --package=bun -- bun scripts/sync_remote_sdk.mjs /path/to/codepet-remote
npm exec --yes --package=bun -- bun scripts/sync_remote_sdk.mjs /path/to/codepet-remote --check
```

同步包含 Gateway、保留旧包名的配对 DTO、channel-control 三套 SDK 及 lock，三份 protocolDigest 应一致。禁止手改 Remote 的生成文件。Dart 生成器要对 optional+nullable 类型的 factory 校验和序列化分别消除重复 null 分支，不以忽略分析 warning 代替修复。

Remote 执行 flutter pub get、flutter analyze、flutter test、flutter build apk --debug；桌面执行正常 Tauri app build，使用当前工作区 provider staging 和公开 SDK 导出。macOS Tauri CLI 会补通用 macos-private-api feature，构建后只恢复 CLI 加入的通用项，保留原 macOS target 声明。

## 独立测试环境

先保存用户原 settings.json 和已安装 App，退出原进程。测试数据目录与用户数据分开；用新 App 的相同可执行文件制作本地签名 QA 包，仅加载 SDK 作者独立实现的 jsonl、zstd、mux Echo Provider。不要在这条自动测试中查询个人 Provider 或修改真实会话。

临时将现有 data.dataDirectory 指向 QA 数据目录，并清空 QA 的额外 Provider 目录/信令设置。此项不是新增永久配置接口。结束后退出 QA App，逐字节恢复原 settings.json，启动正式新版 App。保留备份和测试记录，不能让 QA 设置成为用户的新默认。

测试 defines 只包含 Host 公共 device ID、TLS 指纹和 emulator Gateway 地址。可从 QA SQLite 的 host_identity.device_id 与 host_tls_identity.certificate_sha256 只读查询；不导出私钥、credential、signing seed 或用户配置。Android 凭据由产品配对流程签发并保存在原生 secure storage 中。

## 原生联合测试

Remote 的 `integration_test/desktop_host_app_test.dart` 默认跳过，显式 CODEPET_HOST_APP_QA 才运行。入口使用正式的配对 adapter、生成 SDK、ProtocolGatewayClient、Android 原生 TLS/WebSocket/WebRTC、DeviceRegistry 和 CodePetRemoteApp。Host 必须是正在运行的桌面 App，不使用内存模拟 Gateway。

常规入口可以使用 `flutter test integration_test/desktop_host_app_test.dart -d <device> --dart-define-from-file=/path/to/public-test-defines.json`，需要截图回传时使用 `test_driver/protocol_joint_test.dart`。本机 DDS/VM service 连接不稳定，最终使用设备内执行断言并记录结果的入口：

```sh
env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy \
  flutter build apk --debug --target=integration_test/desktop_host_app_test.dart \
  --dart-define=INTEGRATION_TEST_SHOULD_REPORT_RESULTS_TO_NATIVE=false \
  --dart-define-from-file=/path/to/public-test-defines.json
env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy \
  flutter run --no-pub --use-application-binary=build/app/outputs/flutter-apk/app-debug.apk \
  -d 127.0.0.1:5555 --no-resident --no-dds
adb exec-out run-as com.codepet.remote cat app_flutter/protocol-qa-result.json
```

`flutter run` 退出只表示启动完成。必须等当前这次设备日志出现 `All tests passed!`，并读取本次测试写出的 `{"passed":true,"failures":[]}`。旧结果不能作新运行证据；复跑前应备份并移除上一次设备结果。设备还会写出四张 `app_flutter/protocol-qa-*.png`；普通 APK 重新安装前保存这些结果。

在隔离桌面 App 中确认 Protocol QA Android 的待配对申请。用例验证准确的三个测试 Provider，能力/选择一致性、发送、delta 与完整历史、limit=1 更早页、输入队列查询、应用心跳、两种通道重连、生产 UI 恢复、输入，以及两种通道的凭据撤销。截图仅含合成测试内容。

ECHO_CHUNK_SIZE=4096、ECHO_TURN_DELAY_MS=1 可控制大文本的 delta 数量。chunk 是 JavaScript UTF-16 单元，插件不得切开 surrogate pair；每条 delta 都应可独立被标准 JSON 解码。真实客户端必须保留能力校验，不能为通过 Echo 测试而忽略无效 selection。

## 环境排查与清理

- 本机代理会干扰 Flutter tester 的 loopback HTTP/WebSocket，表现为测试文件全部加载失败。对当前测试进程设置 NO_PROXY/no_proxy 或移除代理环境；不要更改系统代理或放宽 TLS pin。
- 本机旧 Emulator 37.1.11 在 macOS 27 上发生 QEMU 线程挂起，独立下载 Google 官方稳定 37.2.13 并核对 repository checksum 后，新 AVD 可启动。新工具放临时目录，不覆盖用户 SDK；不要通过清空用户 AVD 解决。
- 启动后等待 sys.boot_completed 和 adb device，DDS 尚未就绪时不把 instrumentation 启动失败记为业务失败。关闭 DDS 可能触发当前 Flutter golden 自定义 stream 的参数错误，不能将该错误忽略后宣布通过。
- 原生测试结束后重新构建普通 main.dart APK并安装，避免把 integration_test 入口当作正式 App 交付。按需撤销测试 credential、移除测试 registry，恢复桌面原配置，核对最终 SDK 摘要和 App 签名。

## 验证记录

2026-11-12，API 36 ARM64 模拟器 `CodePet_Protocol_QA_20261112` 与新构建的 macOS App 联调通过。设备日志 16:02:06—16:03:15、PID 8798 覆盖三种 profile × LAN/WSS 与原生 WebRTC，两个实际应用心跳，生产 UI 手动加载更早消息、中文/emoji 发送、切换同 Provider 会话隔离，以及同时撤销两种连接的 credential。最终结果 `passed: true`、`failures: []`。Echo 未实现最近会话能力，首页正确显示不支持；本次不能宣称该能力通过真实 Host 联调。

联调还暴露独立插件的选择默认值、分页游标和 UTF-16 分块问题。由只收到公开 SDK 的独立插件作者修复，未让其读取 Host 源码：源码 19 项及四份发布包各 19 项通过。Host 的三 profile 与跨 Provider 不互相阻塞测试 2 项通过；独立 Pi 的已配置 DeepSeek Flash 真实调用另行执行，1 项通过。这条真实模型调用不是手机 UI 中的模型会话验收。

生成与回归：`npm run protocol:check` 32 项、`npm run providers:test` 5 项、cp-sdk-gen 6 项通过；Remote 三份 SDK `--check` 一致、`flutter analyze --no-pub` 无 issue、全量 `flutter test --no-pub` 503 项通过。两端正常 App 均重新构建。macOS 新包已安装到 `/Applications/Code Pet.app`，本地签名校验通过；原配置逐字节恢复并在正式 App 界面确认使用原数据目录。Android 已重建普通 `lib/main.dart` debug APK 并重新安装，不交付测试入口。

本机验收产物位于 `/Users/wangxin/Documents/CodePet-builds/protocol-joint-20261112/artifacts/`：普通 `CodePet-Remote-debug.apk`、设备结果、四张 UI 截图及 native-test-milestones.log。上层的原配置备份与隔离数据库只用于恢复，不能随验收产物分发。桌面构建日志 `/tmp/codepet-apps-desktop-final-build.log`，Android 构建日志 `/tmp/codepet-apps-android-final-build.log`。模拟器验收不覆盖实体手机、移动数据、公网信令或 TURN；macOS 包为本地签名，未公证。

## v2 前向兼容验收（2026-11-14）

业务 Provider Initialize 与 Gateway handshake 均使用 version/minVersion/selectedVersion，当前范围 2/2，v1 源与视图冻结。三种 stdio profile、配对格式和 WSS `/remote/v1/gateway` 路径保留原有版本。区间改变不代表底层传输改变。

本轮新构建桌面 App、普通 Android App，以及独立作者仅依据公开 SDK 实现的四个 v2 插件包。源码及各插件包 19/19 通过；公开 SDK 标准 Ajv 预检验证 33 个请求、33 个响应和 13 个事件。导出器必须保证所有 `$id` 在同一发布版本命名空间，否则相对 `$ref` 在标准校验器中会失效；仅验证磁盘相对文件存在不足以发现这一错误。

自动检查：协议生成 38 项、发布/构建 5 项、cp-sdk-gen 6 项通过；Rust 主 workspace 472 项通过，14 项外部环境用例按既有条件忽略；Rust SDK 92 项通过，5 项按既有条件忽略；手机静态检查无 issue、506 项测试通过；桌面卡片相关前端 5 项通过。单独启用的 Pi 实测 1 项通过，使用已配置 DeepSeek Flash 读取合成 proof.txt，覆盖实际 RPC、read 工具、delta 和完整历史，未打印密钥。

首次联合运行 PID 20932，17:27:24—17:28:58，完整通过 LAN/WSS 与原生 RTC × 三 profile、两种通道心跳、生产 UI 更早历史/中文 emoji 发送/会话隔离和凭据撤销，设备结果 passed=true、failures=[]。最终构建复验与恢复记录继续写在此节下，不能拿第一次结果冒充后续运行。

产物目录为 `/Users/wangxin/Documents/CodePet-builds/protocol-v2-joint-20261114/`，artifacts 仅存普通 App、公开 SDK、截图和合成测试结果；original-settings.json、QA 数据库和临时配置不随产物分发。原用户配置必须逐字节恢复。

环境：本机代理会使内置 OpenCode 的 localhost HTTP fixture 超时；对测试进程移除代理环境后内置 Provider 4 项通过，不改系统代理。QA 放 Documents 时 sample 曾显示主线程停在 rename/chmod；同一最终二进制改用 `/tmp/codepet-protocol-v2-qa-data-20261114` 后正常启动，未改 TCC、证书校验或安全策略，底层停顿原因未确认。macOS 路径断言应规范化实际存在的包目录，再拼接未必存在的 fixture executable，不能对未创建的文件做 canonicalize。


最终复验 PID 21210，17:35:59—17:37:53，同样完整通过上述六组通道/profile、两次应用心跳、UI 更早历史/发送/会话隔离和双连接撤销；最终设备文件 `protocol-qa-result.json` 为 passed=true、failures=[]，四张截图及本轮日志已保存到 artifacts。原用户 settings.json 已逐字节恢复，原先运行的桌面 App 已重新启动；本次新桌面正常包单独交付，没有覆盖用户原安装或迁移其自定义插件。Android 已重新构建普通 main.dart debug APK 并安装启动，测试入口未作为交付安装包。

交付桌面包为本地签名、未公证；Android 为 debug 签名。本轮覆盖本机模拟器局域网和本地 RTC，不覆盖实体手机、蜂窝、公网 TURN 或发布签名流水线。独立真实模型验证只对指定 Pi 合成任务，不泛化为所有模型或所有 Provider 均通过真实生产账号验收。
