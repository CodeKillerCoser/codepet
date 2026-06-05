# 模块重组

## 规则

目录重组必须保留公开模块契约，更新文档路径，并运行能覆盖编译路径的检查。

## 适用场景

- 移动前端入口文件或共享库。
- 将 Rust 模块移动到新的领域目录。
- 重命名 README、prompts、测试或 include macro 引用的路径。

## 反例

把 `src-tauri/src/hooks.rs` 移到 `src-tauri/src/agent/hooks.rs`，但没有更新 `include_str!`，会因为相对 hook 脚本路径变化而破坏编译。

## 推荐做法

对已跟踪文件使用 `git mv`，更新入口点和 config include；当测试或公开 API 依赖旧模块名时，保留兼容 re-export；移动后搜索陈旧路径。

## 来源

Code Pet 前端 `src/` 到 `frontend/` 的改名，以及 Tauri 模块重组。

## 验证方式

- `npx vitest run`
- `npm run build`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `git diff --check`
