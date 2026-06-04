# Window System

## Current Behavior

The pet overlay window is not user-resizable. `frontend/PetApp.svelte` calls `getCurrentWindow().setResizable(false)` on mount.

The frontend still controls runtime sizing. It calls `ensureWindowSize()` and uses a preset logical width and height before docking or constraining the window. This preserves a stable maximum transparent area while avoiding user-driven resize.

## Monitor Bounds

`frontend/PetApp.svelte` uses `availableMonitors()`, `primaryMonitor()`, `outerPosition()`, and `outerSize()` to select the monitor with the largest window intersection. If there is no intersection, it chooses the monitor nearest the window center or the primary monitor fallback.

`clampWindowPositionToMonitor()` clamps the pet window to the selected monitor work area so it does not exceed screen edges.

## macOS Overlay Setup

`src-tauri/src/platform/macos_window.rs` configures macOS-specific overlay behavior. The panel can become key only when needed so reply inputs can receive focus while keeping the pet window as a floating overlay. Non-macOS builds use the no-op wrapper in `src-tauri/src/lib.rs`.

## Risks

- Cross-screen dragging can briefly produce stale position or size data.
- Changing content height without reviewing `ensureWindowSize()` can reintroduce clipped task lists or unnecessary scrollbars.
- Monitor work areas may use negative coordinates on multi-monitor setups.

## Validation

- Verify `npx vitest run` for frontend layout logic indirectly covered by tests.
- Manually test dragging across monitors on Windows and macOS when changing `PetApp.svelte` window code.
