# 主题系统

这个领域覆盖项目主题库，以及从写死 UI 颜色迁移到 token 的规则。

## 源码模块

- `frontend/lib/theme/tokens.css`：primitive、semantic、component 和 asset CSS token。
- `frontend/lib/theme/defaults.ts`：TypeScript 默认值和主题 class 名。
- `frontend/lib/theme/index.ts`：主题公开导出。
- `frontend/lib/theme/README.md`：主题库规则。
- `src-tauri/src/pet/theme_defaults.rs`：与持久化设置对齐的 Rust 默认值。

## 原则

生产 UI 样式应使用主题 token。用户可编辑颜色可以继续作为持久化数据存在，但默认值应来自主题库。
