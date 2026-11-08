# 窗口系统

## 当前行为

桌宠复用一个 pet WebView/NSPanel，包含 floating 和 docked 两种模式。Svelte 负责角色、任务和内容测量；原生 `pet::surface` 是窗口位置/尺寸的唯一写入者，前端不再直接 setSize/setPosition。用户手动缩放仍禁用。

`frontend/lib/pet/surface.ts` 观察自然内容高度，连同角色中心、角色尺寸和布局 revision 发送原生。异步旧布局不能覆盖新模式或拖动结果。桌面 UI 宽 320px、高度贴合内容，固定角色中心；顶部 UI 收起按内容自适应、宽至多 280px、高 44px；hover 展开至多 640px 的深色面板，顶栏与列表同宽且固定不滚动。停靠按面板中心对齐顶部，下缘定位不因左侧头像偏移。见[布局与尺寸](../30-domains/pet-window/layout-and-sizing.md)。

两套具体组件 `DesktopPet` 和 `NotchPet` 共用 `model.ts` 数据与 `presentation.ts` 状态转换，不定义通用 UI 接口。原生 Surface 管理窗口事务，前端只管理展开、焦点和桌面收起。见[两套 UI 与共享状态](../30-domains/pet-window/two-views-and-shared-state.md)。

## 屏幕与拖动

`src-tauri/src/platform/pet_surface.rs` 提供统一平台接口。macOS 从 NSScreen 的 safeAreaInsets/auxiliaryTopArea 读取真实刘海，普通外接屏回退顶部居中停靠。AppKit 点坐标在边界转换成向下为正的坐标，不混用 Tauri 物理像素。其他平台保留原生窗口拖动，不提供刘海能力。

原生拖动过程暂不按工作区钳制；角色锚点进入顶部候选区后显示提示，松手才提交停靠。退出候选区有滞回。macOS 本地鼠标监视器只处理角色区域的拖动；全局只监听松手，配合原生指针状态结束跨窗口拖动。Escape 恢复拖动前模式和锚点。

窗口状态单独存到 data/config/pet-surface.json，保存模式、显示器和归一化锚点。启动/前端热加载读取当前原生状态，不再无条件回到右下角。无配置时仍使用右下角默认位置；显示器移除时回退可用屏幕。

## macOS 悬浮设置

`platform/macos_window.rs` 仍配置非激活 NSPanel、跨 Space、无阴影和按需成为 key window。Surface 适配器只在顶部停靠或拖动时提高层级。透明像素和非交互区域仍通过窗口级 ignoresMouseEvents 透传；不采用全屏宽的大透明 frame 或鼠标事件重投。

新增原生命令及面板居中语义使 webcontent backendApiVersion 升至 6（上一版吸附预览为 5）。需要先升级原生 App，之后布局与素材继续支持独立前端更新。

## 验证

纯几何/状态测试覆盖角色锚点、负坐标、缩放、吸附滞回、松手提交、取消、拔屏和配置保存。前端控制器测试覆盖旧布局回复、快速松手、卸载清理；浏览器验证内容尺寸、向下展开、键盘焦点、按钮可见性和响应尺寸。原生实机验证范围与版本见[刘海接入设计及实施记录](../30-domains/pet-window/notch-docking-design.md)。
