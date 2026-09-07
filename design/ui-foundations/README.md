# 已确认的 UI 视觉稿

2026-09-08 定稿。设计规范见 [全局 UI 视觉基础提案](../../knowledge/20-product/ui-foundations-proposal.md)。

- [macOS 交互稿](visual-macos.html)：左上角红黄绿窗口控制。
- [Windows 交互稿](visual-windows.html)：右上角最小化、最大化与关闭。
- [可编辑源稿](visual-source.html)：交互片段源文件；以上两个完整 HTML 从同一份源稿生成。

两端统一采用顶部工具栏与导航共享背景、16px 圆角内容面板、36px 品牌区。主面板无阴影，装饰边框仅约 3% 不透明度；控件边界与焦点环独立保留。

直接用浏览器打开完整 HTML。图标等预览资源首次加载需要网络。源稿中的平台／场景调节依赖会话预览宿主；完整 HTML 已分别固定初始平台，内部主题切换、详情、筛选等交互仍可用。窗口按钮仅为示意，示例数据与操作不连接真实服务。

重新生成时使用 visualize 技能的 `scripts/render.py visual-source.html visual-macos.html --force`。Windows 版本只需在源稿副本中将根元素的 `data-platform` 和 `platformState.platform` 初始值改为 `windows`，再调用同一导出脚本；CSS 中匹配 macOS 的选择器必须保留。

`visual-board.png` 为历史探索稿，不再作为实现依据。`color-tokens.json` 和 `contrast-check.json` 为早期颜色快照，未覆盖最终壳层色值；生产主题迁移时需同步并重新验证。
