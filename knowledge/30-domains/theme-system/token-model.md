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
| 主标题 | `--font-title` / `--font-size-title` | 20px |
| 二级标题 | `--font-section` / `--font-size-section` | 18px |
| 三级标题 | `--font-subheading` / `--font-size-subheading` | 16px |
| 正文 | `--font-body` / `--font-size-body` | 15px |
| 控件文字 | `--font-size-label` | 14px |
| 备注与元信息 | `--font-caption` / `--font-size-caption` | 13px |
| 路径、代码 | `--font-size-code` + `--font-family-code` | 13px |

语义字体在 `:root, .main-theme` 中声明，使主窗口覆盖的字体族和字重在当前主题元素上重新计算。只在 root 定义复合 font token 再继承，会提前解析 root 的字体族，导致正文使用桌宠字体。既有紧凑桌宠字号保留 primitive token，禁止在组件里重新写数字。

字重只保留三个原始值：content/regular 为400、emphasis/medium 为500、heading/semibold 为600。旧 normal、medium-strong、bold、heavy 等名字只作这三档的别名，不再各自定义数值；控件为400、激活项为500。主窗口 strong/b/th 显式复用强调字重，避免浏览器默认700穿透层级。结构标题由上到下不递增：App品牌20 ≥ Task标题20 ≥ 执行分组18 ≥ 执行标题16 ≥ 正文15；侧栏标题18 ≥ 分区16 ≥ 验收点15，正文均400。

间距使用 space-inline/related/block/section/page：4/8/12/28/24px。同组标签、内容、元信息使用4–8px，字段或事件相邻12px，独立分区28px。长文字不通过加粗或加大字号解决，使用换行、完整展开和组件内对齐。

边框沿用 Radix Gray：4 阶分隔与容器轮廓、6 阶控件默认边框、8 阶悬停或增强边框。主窗口不以纯黑或透明黑叠加模拟边框，暗色主题由对应 gray palette 提供。状态以图标及文本表达，不依赖大面积背景色。

## 按钮交互

主窗口按钮统一采用无色或中性灰背景。`frontend/lib/theme/main-window.css` 定义 `--hover_background_color: var(--gray-3)`、`--press_background_color: var(--gray-4)`；Radix 明暗 palette 自动适配主题。原 `--color-main-hover`、`--color-main-selected` 分别作这两个值的别名，主操作、选中导航和工具栏也使用灰阶，不单独使用黄色。

`frontend/main-window.css` 统一提供可用按钮 hover、鼠标按下及持久选中背景，覆盖主内容、全局侧栏和 portal 弹窗。disabled/aria-disabled 不产生新反馈；switch 的 checked 轨道保留独立状态语义。按钮形状、默认透明或浅灰背景仍由组件承担，成功/失败等内容状态保留语义色。Cognee 沙箱内的上游渲染不读取主窗口样式。

该变更涉及主题角色和公共交互层，避免时钟关闭后变灰、展开后仍出现黄底，或主按钮悬停回退为黄色。修改后检查明暗主题下的时钟关闭/展开、hover/按下/释放、侧栏操作、设置主按钮、disabled 与全局弹窗，验证实际 computed style，不只检索变量名。

## 验证

- 修改 token 时运行 `npx vitest run frontend/styles.test.ts` 和主题相关测试。
- `frontend/styles.test.ts` 同时检查所有生产组件 style 块的字体数值，覆盖 font 简写；语义字体是否使用主窗口字体族由任务 refinement UI 验证。
- 完成 UI 颜色工作前，搜索 `frontend/lib/theme/` 之外的 raw color literal。
