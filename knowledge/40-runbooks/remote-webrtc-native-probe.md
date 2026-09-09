# WebRTC 原生互通验证

## 适用范围

验证 Windows Rust Host 与 Android flutter_webrtc 的原生 DataChannel。
此探针使用一次性配对凭据和独立 Android 包 `com.codepet.remote.rtcprobe`，
不覆盖用户的正式 Remote。HTTPS 信令可经 ADB reverse，RTC 数据仍经 ICE
建立的网络路径；手机与电脑需有可互达地址。本探针不验证公网信令或 TURN。

## 前置条件

两个实施工作树、Rust/Flutter/Android 工具链可用；手机允许 USB 调试，
`adb devices -l` 显示 device。安装数据通道依赖后运行 `flutter pub get`。
按用户最新要求，Debug 安装包使用 Remote 仓库固定的 `android/keystore/codepet-debug.keystore`；
构建时不得重新生成 key。Release 单独使用 CODEPET_RELEASE_*。测试包名固定为上述 rtcprobe，正式包名不变。
测试不申请音视频权限；QR 扫码原有相机权限属于原产品，与 RTC 无关。

## 步骤

在 Host 工作区的终端启动一次性服务：

```powershell
$env:CODEPET_RTC_PROBE_EXPORT=Join-Path $env:TEMP 'codepet-rtc-probe.json'
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test remote_lan_listener rtc_android_probe_host -- --ignored --exact
```

服务将临时配置写入指定文件，不向控制台输出凭据；10 分钟自动退出。
文件包含当前测试的 credential、pin、端口和生成协议请求，不应提交或分享。
每次重新运行探针要启动新的服务，成功测试会撤销旧凭据。

在 Remote 工作区的另一终端运行（按 adb 输出设置设备 ID）：

```powershell
$probeDevice = '设备ID'
$probePath = Join-Path $env:TEMP 'codepet-rtc-probe.json'
$probeConfig = Get-Content -Raw -LiteralPath $probePath | ConvertFrom-Json
$probePort = ([uri]$probeConfig.RTC_GATEWAY_URI).Port
adb -s $probeDevice reverse "tcp:$probePort" "tcp:$probePort"
$env:CODEPET_RTC_PROBE = 'true'
flutter test integration_test/rtc_gateway_test.dart -d $probeDevice --dart-define-from-file=$probePath
```

USB reverse 只映射信令监听端口，不将 UDP/SCTP 数据通道装入 USB。
当前用例检查握手、>64 KiB UTF-8 请求/响应、协议 ping、订阅后事件和撤销断连。
Host 普通集成测试另外覆盖同凭据 LAN/RTC 并存与同时撤销。

## Windows 跨盘 Kotlin 缓存问题

2026-09-09 在 C 盘工作树、D 盘 pub 缓存下，device_info_plus 和 flutter_webrtc
的 Kotlin 增量缓存出现 `different roots` / `Could not close incremental caches`。
通过 Gradle project 环境变量传递禁用参数未解决；在本次构建期间临时加入以下
参数后构建成功，结束后恢复原文件。不要把开发机规避参数无条件提交到项目：

```powershell
$rtcPropsPath = (Resolve-Path 'android/gradle.properties').Path
$rtcOriginalProps = [System.IO.File]::ReadAllBytes($rtcPropsPath)
$rtcProbeExit = 1
try {
  Add-Content -LiteralPath $rtcPropsPath -Value "`nkotlin.incremental=false`nkotlin.compiler.execution.strategy=in-process"
  flutter test integration_test/rtc_gateway_test.dart -d $probeDevice --dart-define-from-file=$probePath
  $rtcProbeExit = $LASTEXITCODE
} finally {
  [System.IO.File]::WriteAllBytes($rtcPropsPath, $rtcOriginalProps)
}
if ($rtcProbeExit -ne 0) { throw 'Native RTC probe failed; inspect test output' }
```

保留测试进程的 exit code，不能因为 finally 恢复文件成功就把测试失败标为通过。
插件当前还会报告 KGP 未来迁移提示；该提示本身不代表本次构建失败。

## 清理与结果记录

```powershell
New-Item -ItemType File -Path "$probePath.stop" -Force | Out-Null
adb -s $probeDevice reverse --remove "tcp:$probePort"
adb -s $probeDevice uninstall com.codepet.remote.rtcprobe
Remove-Item Env:CODEPET_RTC_PROBE -ErrorAction SilentlyContinue
```

Host 观察 stop 文件后关闭会话并删除临时配置/stop 文件。确认服务终止，
`git diff -- android/gradle.properties` 无变化。记录测试版本、设备、网络、
命令和实际通过项到 [实施与验收](../20-product/webrtc-channel-delivery.md)。

## 开发模式选择

普通 Remote 构建继续使用 LAN/WSS。要使用新增 RTC adapter，
在 Remote 的 App 设置打开“开启 WebRTC”，完全退出并重新打开 App；仍先按已有 LAN 流程配对，
沿用原证书 pin 和凭据。应用组合入口选择 transport，界面和业务 SDK 不改。

设置页显示本次启动的当前通道。设置已替代原 CODEPET_WEBRTC 编译期开关，默认关闭；
关闭并重启恢复 LAN/WSS，当前会话不随开关即时切换。
该开关只选择 LAN 信令下的 RTC，不代表完成公网 rendezvous。当前 adapter
没有 STUN/TURN 配置，文件挂载/传输/预览不在本轮范围。
