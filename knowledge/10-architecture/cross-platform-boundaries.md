# 跨平台边界

## macOS

macOS 拥有最完整的平台能力：

- `src-tauri/src/platform/macos_window.rs` 负责悬浮窗口配置。
- `src-tauri/src/agent/actions.rs` 支持通过 bundle id、应用名、Terminal 和 iTerm 路径激活目标。
- README 记录了 DMG 打包和签名脚本。

## Windows

Windows 支持核心 Tauri/Rust 编译、hook 配置、本地 collector 行为和 `.ico` 资源。部分 macOS 专属激活路径会明确返回不支持错误。

Windows hook 命令转义由 `src-tauri/src/agent/hooks.rs` 的 Windows 分支处理，包括 `%` 和引号转义。

## Linux

Linux 需要 Tauri/WebKitGTK/GTK 系统依赖。README 也说明了在 macOS 上交叉检查 Linux target 时需要额外 sysroot 和 `pkg-config` 配置。

## 规则

平台专属能力不能假装可用；无法可靠执行时应明确失败。当前端按钮依赖 provider 或平台能力时，应根据后端能力隐藏或禁用。
