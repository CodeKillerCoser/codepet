# Cross Platform Boundaries

## macOS

macOS has the richest platform behavior:

- Overlay window configuration in `src-tauri/src/platform/macos_window.rs`.
- Activation through bundle id, app name, Terminal, and iTerm paths in `src-tauri/src/agent/actions.rs`.
- DMG packaging and signing scripts documented in README.

## Windows

Windows supports core Tauri/Rust compilation, hook configuration, local collector behavior, and `.ico` resources. Some macOS-specific activation paths intentionally return unsupported errors.

Windows hook command quoting is handled by the Windows branch in `src-tauri/src/agent/hooks.rs`, including escaping `%` and quotes.

## Linux

Linux requires Tauri/WebKitGTK/GTK system dependencies. README notes that macOS cross-checks for Linux require extra sysroot and `pkg-config` setup.

## Rule

Platform-specific capability should fail explicitly instead of pretending to work. UI buttons should be hidden or disabled by provider capability when the backend cannot reliably execute the action.
