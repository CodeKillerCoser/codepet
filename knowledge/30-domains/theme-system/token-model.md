# Token 模型

## 层级

前端主题库使用以下层级：

1. `frontend/lib/theme/tokens.css` 中的 Radix primitive palette。
2. 语义 token，例如 `--color-*`、`--font-*`、`--line-height-*` 和 `--letter-spacing-*`。
3. component alias，例如 `--app-*` 和 `--pet-*`。
4. asset token，例如 `--asset-*`。

## 主题 Class

`frontend/lib/theme/defaults.ts` 暴露 `themeClassNames()`，返回亮色或暗色模式需要的 class 集合。`frontend/App.svelte` 和 `frontend/PetApp.svelte` 会根据设置和系统偏好应用这些 class。

## 内容字体层级

主窗口内容使用语义字体，字号、字重、行高均由主题定义；小窗口标题使用 compact 别名。

| 层级 | 字体 token | 字号 |
| --- | --- | --- |
| 主标题 | `--font-title` / `--font-size-title` | 24px |
| 二级标题 | `--font-section` / `--font-size-section` | 18px |
| 三级标题 | `--font-subheading` / `--font-size-subheading` | 16px |
| 正文 | `--font-body` / `--font-size-body` | 15px |
| 控件文字 | `--font-size-label` | 14px |
| 备注与元信息 | `--font-caption` / `--font-size-caption` | 13px |
| 路径、代码 | `--font-size-code` + `--font-family-code` | 13px |

语义字体在 `:root, .main-theme` 中声明，使主窗口覆盖的字体族和字重在当前主题元素上重新计算。只在 root 定义复合 font token 再继承，会提前解析 root 的字体族，导致正文使用桌宠字体。既有紧凑桌宠字号保留 primitive token，禁止在组件里重新写数字。

边框沿用 Radix Gray：4 阶分隔与容器轮廓、6 阶控件默认边框、8 阶悬停或增强边框。主窗口不以纯黑或透明黑叠加模拟边框，暗色主题由对应 gray palette 提供。状态以图标及文本表达，不依赖大面积背景色。

## 验证

- 修改 token 时运行 `npx vitest run frontend/styles.test.ts` 和主题相关测试。
- `frontend/styles.test.ts` 同时检查所有生产组件 style 块的字体数值，覆盖 font 简写；语义字体是否使用主窗口字体族由任务 refinement UI 验证。
- 完成 UI 颜色工作前，搜索 `frontend/lib/theme/` 之外的 raw color literal。
