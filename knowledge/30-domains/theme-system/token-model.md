# Token 模型

## 层级

前端主题库使用以下层级：

1. `frontend/lib/theme/tokens.css` 中的 Radix primitive palette。
2. 语义 token，例如 `--color-*`、`--font-*`、`--line-height-*` 和 `--letter-spacing-*`。
3. component alias，例如 `--app-*` 和 `--pet-*`。
4. asset token，例如 `--asset-*`。

## 主题 Class

`frontend/lib/theme/defaults.ts` 暴露 `themeClassNames()`，返回亮色或暗色模式需要的 class 集合。`frontend/App.svelte` 和 `frontend/PetApp.svelte` 会根据设置和系统偏好应用这些 class。

## 验证

- 修改 token 时运行 `npx vitest run frontend/styles.test.ts` 和主题相关测试。
- 完成 UI 颜色工作前，搜索 `frontend/lib/theme/` 之外的 raw color literal。
