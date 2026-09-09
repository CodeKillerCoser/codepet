# WebRTC 原生依赖独立构建与打包

## 背景与目标

2026-09-09，现有 Host 固定使用 webrtc-rs 0.14.0，其 TURN/TCP、TURN/TLS 候选采集
未实现。用户要求把替代方案 libdatachannel + libnice 的下载、工具链、构建与缓存独立
封装，Rust App 只请求产物。本次完成该边界，不修改 JSON-RPC、LAN、配对或手机协议。
本次也不以“原生库编译成功”代替后续通道适配和 TURN/TLS 实网验收。

## 决策与目录

源码核查发现新的选型阻碍：libnice 0.1.23 `agent/agent.c::agent_create_tcp_turn_socket`
对 TURN_TLS 仅在 GOOGLE/OC2007 兼容模式添加 pseudossl；RFC5245 模式没有标准 TLS
握手。libdatachannel `IceTransport::addIceServer` 仅传入 NICE_RELAY_TYPE_TURN_TLS，
没有额外封装。因此这个固定版本组合不能解决最初的 TURN/TLS 需求，不能依据 DOC 中
列出 `tls` 参数就宣称支持。版本清单已明确记录 `unavailableFeatures: ["turn-tls"]`。
已向用户说明并征询后续后端方向；独立产物契约仍可复用。

`tools/webrtc` 是独立生产者：`source/` 放固定 revision 的 libdatachannel 和 vcpkg
Git checkout；`cache/` 放下载、工具和中间产物；`out/<target>/<key>/` 发布 SDK。
以上生成目录被忽略，仓库仅保存版本清单、triplet、脚本、测试及说明。

使用 libdatachannel 0.24.3 + 固定 vcpkg revision 的 libnice 0.1.23，显式 `USE_NICE=ON`，
关闭音视频、WebSocket、示例和上游测试。共享库方式保留可替换的依赖边界，避免 App
自行维护传递链接参数。vcpkg 统一构建 OpenSSL/libnice/GLib 等依赖；不使用机器上
未经版本约束的 Homebrew 或全局 vcpkg 库。

## 构建契约与缓存

消费者执行 `node tools/webrtc/build.mjs --target <Rust target triple>`，stdout 返回
SDK 目录与 `artifact.json` 路径，日志走 stderr。manifest 包含版本、目标、编译器记录、
include/lib/runtime 路径及所有导出文件的 SHA-256。`--check` 只校验，缺失时明确失败。

版本、架构、triplet 或 recipe 不一致会更换缓存 key。命中时逐文件校验，不访问网络，
也不探测编译器。Debug App 同样使用 Release 原生库。缓存不自动随本机兼容编译器
升级失效；需要改变 ABI 或最低工具链要求时必须更新 triplet/recipe，让版本要求显式化。

整个共享构建目录用 PID 锁串行保护。失败不写完成 manifest，成功后重命名发布；清理
仅允许模块生成目录。保留不同版本产物，避免升级时破坏其他工作进程。

## 涉及模块与打包

- `tools/webrtc/`：拥有原生源码、工具链发现、依赖解析、验证与产物生命周期。
- `src-tauri/build.rs`：按 Cargo TARGET 请求 SDK 并导出 manifest 路径，不含 CMake 逻辑。
- `scripts/tauri_with_native.mjs` / `package.json`：npm Tauri 入口请求产物，生成本次打包配置。
  Windows DLL 进入安装目录，与 EXE 同级；macOS 动态库进入 Frameworks，并由 Tauri 签名。
  动态库不使用 sidecar 的 `externalBin`，头文件、编译器、源码和调试符号不进入安装包。
- `.github/workflows/release.yml`：安装 macOS CMake/Ninja，缓存 SDK、下载和依赖二进制；
  Universal 构建分别请求 arm64/x64 SDK，然后合并 dylib。原有发布与签名流程保留。

macOS 动态库引用改写为 `@loader_path`，拒绝未打包的非系统依赖。两架构分别编译 ABI
探针，仅在当前架构运行；跨架构产物需要另一台对应架构 Mac 或 CI 进一步执行。

## 风险与验证路径

- 缓存损坏、版本混用：缺失/篡改文件、版本和架构变更测试，真实第二次构建验证复用。
- 隐含 DLL/dylib 依赖：C ABI 创建/销毁 PeerConnection 探针；安装目录在干净机器验收。
- Universal 与签名：CI 执行双架构构建、`lipo -info` 和最终 App codesign 校验；本机 Windows
  无法执行 macOS 编译、公证，不把脚本存在记为 macOS 已通过。
- 只构建未消费：目前业务通道仍使用原 Rust 库；后续替换适配器后，再执行禁用 UDP 的
  强制 TURN/TLS、手机互通、大消息、背压和 LAN 回归。

## 验证记录

- `npm run webrtc:test`：缓存与打包契约测试通过。
- Windows 首次原生构建、第二次复用及 App 打包：执行中，结果完成后更新。
- macOS 实机编译、签名、公证及 TURN/TLS 互通：未执行。

## 依据

- [libdatachannel 构建选项](https://github.com/paullouisageneau/libdatachannel/blob/v0.24.3/BUILDING.md)。
- [libdatachannel C API 的 TURN 后端限制](https://github.com/paullouisageneau/libdatachannel/blob/v0.24.3/DOC.md)。
- 固定源码 `tools/webrtc/source/vcpkg/ports/libnice` 声明 OpenSSL 后端，禁用 GStreamer/测试/示例。
