# 主题迁移规则

## 规则

新的生产 UI 样式不应在主题库之外引入 raw hex、rgb、hsl 或命名颜色，除非该值来自用户输入或外部资源要求。

## 推荐做法

- 在 `frontend/lib/theme/tokens.css` 中新增或复用语义 token。
- 更新特定界面时使用 component alias。
- TypeScript 默认值保留在 `frontend/lib/theme/defaults.ts`。
- 当默认值跨越后端边界时，保持 `src-tauri/src/pet/theme_defaults.rs` 中的 Rust 持久化默认值一致。

## 验证

- 使用 `rg` 搜索 `frontend/` 中的 raw color literal。
- 主题迁移后运行前端测试。
