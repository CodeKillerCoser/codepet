# 平台依赖隔离

## 规则

平台专用依赖及其 feature 放在 Cargo 对应的 target.dependencies 中，调用点和模块入口使用同一平台条件。不要在公共依赖上开启 macOS 私有 API。

编译隔离与依赖解析是两件事：Cargo 为统一锁文件解析其他 target 的依赖。`cfg` 和 optional feature 都不能保证 Windows 不访问 macOS 的 Git 源。

## 适用场景

Tauri 原生窗口、系统身份、图像能力、IPC，以及原生平台 SDK 的新增和升级。

## 反例

- 公共 `tauri` features 包含 `macos-private-api`，使平台能力边界不明确。
- 将 `tauri-nspanel` 放入 macOS target 后，以为 Windows 离线构建不再需要该 Git revision；2026-09-06 本机测试仍在解析阶段被缺失的 Git 缓存阻塞。

## 推荐做法

- `src-tauri/Cargo.toml` 的 macOS 区域持有 Apple 框架、`tauri-nspanel` 和 Tauri 的 `macos-private-api`；Unix 区域持有 `libc`，对应 `connect_validated_socket` 的 `cfg(unix)`。
- `app.macOSPrivateApi` 只在 `tauri.macos.conf.json` 中开启；公共 `tauri.conf.json` 不再开启它，避免 CLI 按公共配置重新启用平台 feature。
- 保留单一 Tauri 构建入口和共享源码。固定提交的 `tauri-nspanel` 放在 `src-tauri/vendor/tauri-nspanel`，通过 macOS target 的 path 依赖引用，避免 Git 拉取阻塞其他平台。源码未改动，保留两份许可证；来源、提交及 manifest 裁剪记录在 `UPSTREAM.md`。
- 这不是完全独立的锁文件或注册表依赖图。Cargo 仍解析各平台的注册表依赖；Windows 编译树不应包含 macOS 专用库或 feature。
- Windows 构建工具也应定位原生可执行文件：npm 全局 Bun 的 PATH 入口可能仅有 `.cmd` / `.ps1`。`scripts/stage_provider_plugins.mjs` 通过标准路径 API 定位包内 `bun.exe`，避免 `spawnSync` 直接执行 shim；`BUN` 可以显式覆盖。staging 测试按实际 target 验证后缀，POSIX 权限断言仅在 Unix 主机执行。
- 生成代码必须跨平台稳定。源文件说明先用 `path.relative` 计算，再统一为正斜杠的展示格式；freshness 检查忽略 Git checkout 的 CRLF/LF 差异，但必须拒绝其他内容变化。2026-09-06 Windows release 的协议检查曾因这两项差异误报 stale，Rust/TypeScript 文件仅头部路径变化，Dart 文件仅换行变化。修复位于 `tools/protocol-codegen/generate.mjs`，不改变协议或 DTO。
- 升级 vendored 库时对比上游固定提交、更新来源和锁文件，不直接修改第三方实现来绕过平台错误。

## 来源

- [Windows 平台审阅](../10-architecture/windows-platform-review.md)
- [Cargo dependency resolver](https://doc.rust-lang.org/cargo/reference/resolver.html)
- [tauri-nspanel 固定提交](https://github.com/ahkohd/tauri-nspanel/tree/a3122e894383aa068ec5365a42994e3ac94ba1b6)

## 验证方式

`cargo metadata --no-deps --manifest-path src-tauri/Cargo.toml` 检查直接依赖的 target 和 feature 归属。运行 `cargo tree --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc -e normal,build,features`，确认没有 tauri-nspanel、objc2 或 macos-private-api；macOS target 的树应保留这些依赖。再运行对应平台 cargo check/test。

本机是 Windows，macOS 的实际编译与窗口验证仍需 macOS 主机：检查桌宠浮动层级、全屏空间可见性和点击不抢焦点。源码逐文件对比只能验证 vendoring 未改逻辑，不能替代运行验证。

2026-09-06 验证：metadata 的 target/feature 归属通过；Windows 依赖树没有上述 macOS 专用库或 Tauri feature，`cargo tree --offline --locked ... --target x86_64-pc-windows-msvc -i tauri-nspanel` 返回空树；`cargo tree ... --target aarch64-apple-darwin -i tauri-nspanel` 保留 code-pet → 本地 tauri-nspanel。公共/macOS JSON 配置检查通过；5 个上游 Rust 文件和 2 份许可证与固定提交逐字节相同。

正常配置下 Windows `event_normalizer_tests` 14 项通过，`npm run providers:test` 3 项通过，`npm run providers:stage:dev` 通过。Bun 已按用户要求全局安装（1.4.2）。

`npm run protocol:check` 18 项通过；新增临时输出目录用例检查生成头部的正斜杠、CRLF checkout 可通过以及内容改动仍会被拒绝。
