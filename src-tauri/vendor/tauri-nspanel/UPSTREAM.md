# Vendored tauri-nspanel

Source: https://github.com/ahkohd/tauri-nspanel

Commit: `a3122e894383aa068ec5365a42994e3ac94ba1b6` (2.1.0).

The `src/` files and licenses are unchanged. The manifest omits the upstream
workspace and example workspace dependencies; examples are not vendored.

This remains a macOS-only target dependency. Keeping the pinned source locally
avoids Cargo fetching a macOS Git dependency when resolving a Windows/Linux
build. Cargo still resolves target-specific registry packages for the lockfile.

When updating, compare against the pinned upstream source, preserve both
licenses, update this commit and Cargo.lock, and validate the macOS panel
behaviors as well as the Windows dependency tree.
