# 主窗口原生按钮与工具栏对齐

## 规则

macOS 网页工具栏必须使用 AppKit 实际按钮边界作为锚点。导航收起、展开只能改变内容布局，不能移动相对于窗口定位的工具栏按钮。

## 适用场景

主窗口 Overlay 标题栏、侧栏折叠及窗口尺寸变化。Windows 继续使用独立的窗口控制按钮。

## 反例与证据

2026-09-08 用户反馈 macOS 收起按钮没有与红绿灯中心对齐，间距还会变化。旧实现用 70px 占位和 40px 工具栏居中估算位置；`.sidebar-collapsed` 又给整个网格增加 8px 左内边距，因此工具栏会相对原生按钮右移。具体引入提交未复核。

## 推荐做法

- `src-tauri/src/platform/main_window.rs` 在 AppKit 主线程读取原生绿灯边界，转换到 content view 坐标，处理坐标轴方向并返回逻辑点。网页按钮左边缘位于绿灯右边缘外 12px，中心高度与绿灯一致；启动和重新创建窗口共用此查询，无需复制平台位置配置。
- `frontend/lib/windowChrome.ts` 封装平台调用，在初始化及 resize 后读取锚点；`WindowToolbar.svelte` 接收锚点，避免业务页面判断平台。
- `frontend/main-window.css` 为 macOS 单独定位按钮，抵消收起状态的 8px 外层位移，保持 32×28px 命中区域，焦点轮廓向内避免贴近窗口顶部时裁切。
- 增加前进/后退时，以 `.window-navigation-controls` 按钮组整体定位到同一锚点，组内用 flex 排列，不能让每个按钮都 absolute 到同一点。侧栏 padding 动画与工具栏补偿 margin 必须使用相同时间和曲线，减少动态效果时同时关闭，避免过渡期间出现 8px 偏移。对应浏览器回归逐帧检查锚点和按钮互不覆盖。

## 来源

用户 macOS 实际反馈、上述源码，以及原生 `NSView.convertRect:toView:` 坐标转换接口。设计背景见 [全局 UI 视觉基础提案](../20-product/ui-foundations-proposal.md)。旧提案中的 70px 占位验证属于历史实现，已由本次原生几何查询替代。

## 验证方式

- `npx vitest run frontend/lib/windowChrome.test.ts frontend/styles.test.ts frontend/connections.test.ts`：43 项通过，包括 macOS 初始化／resize 几何同步、失败处理和 Windows 不发起几何调用。
- `npm run build` 通过；`cargo check --manifest-path src-tauri/Cargo.toml --offline -j 2` 在 Windows 通过（既有未使用代码警告）。没有运行耗时的完整 Windows 打包。
- 浏览器 macOS 分支在 820×600 与 980×700 下收起、键盘展开均保持相同按钮坐标，内容无横向溢出。浏览器只验证网页几何，不连接原生设置，不能代表 AppKit 验收。
- 剩余验证：在 Mac 实机确认启动、缩放、关闭后重开和全屏进出时的原生中心线及 12px 间距。本机无法编译或运行 macOS 分支，不能宣称这些场景已通过。
