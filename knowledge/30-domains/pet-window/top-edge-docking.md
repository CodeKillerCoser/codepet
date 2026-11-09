# 统一顶部吸附

## 依据与目标

2026-11-10 用户确认统一视觉图：所有吸附位置保留顶部外圆角，默认居中，允许左右移动；物理刘海只影响纵向安全区。保留已有任务列表、自定义角色、拖出悬浮及拖回吸附。视觉目标为 `exec-3722cb08-2621-4cb2-bfcd-002e28636400.png`，不采用此前独立菜单入口或分离胶囊方案。

## 行为与实现

- `pet/surface/geometry.rs`：整个屏幕顶部都是吸附目标；窗口 frame 从屏幕顶边起，水平居中于保存的位置并限制在屏幕边界。与刘海横向相交时返回摄像头安全区；展开后的实际宽度也参与判断，防止收起未相交而展开遮住文字。黑色背景覆盖安全区，所以视觉上与刘海一体。
- `pet/surface/mod.rs`：沿顶部拖动保持 docked，离开吸附带后转为 floating，松手进入顶部目标才回吸。靠近刘海中心时自动对齐；Escape 恢复拖动前屏幕、位置与模式。`dockXRatio` 独立保存窗口横向中心，原有角色锚点继续服务悬浮缩放；旧配置缺少此字段时迁移为 0.5，不沿用旧菜单头像位置。
- `platform/pet_surface.rs`：封装窗口显示/隐藏；删除独立 `pet_menu` 平台模块。窗口等待当前 revision 的测量后显示，避免瞬间出现旧大窗口。
- `NotchPet.svelte`：统一外圆角，在同一黑色容器顶部增加 `topInset`，下方维持单行状态或固定顶栏与滚动任务列表。刘海重叠时最小宽度容纳刘海；已完成/取消分组、每组单图标、6px 内边距和堆叠样式不变。
- `presentation.ts`、`PetApp.svelte`：所有顶部位置统一使用 hover/焦点展开，删除菜单摘要与 native panelOpen 分支；沿顶部拖动保留开始时的展开/收起状态；鼠标移入等待 200ms 后展开，头像按下即取消待展开。原角色导入与共享任务数据不变。

## 契约与交付

当前原生 API 为 12：增加原生画面快照 begin/end 握手；API 11 增加只读 `pet_surface_plan_layout`，为动效提供同一几何算法下的前后窗口 frame；此前增加 `topInset`/`notchWidth`、移除菜单命令与模式字段；本轮增加测量所得 `dragRegion`，让原生事件监视器与前端使用同一顶栏拖动范围。必须安装完整新版；前端包兼容校验阻止旧 API 缓存覆盖新 UI。没有修改 Provider 协议或 Remote 项目。

## 风险及验证

- 定位/持久化：Rust 测试覆盖报告原始刘海几何、无刘海工作区、横向移动、展开保持中心、屏幕边界、旧配置迁移、负坐标及 DPI。
- 拖动：Rust 测试覆盖下拖脱离、任意顶部位置松手回吸、刘海中心吸附、跨屏与 Escape；仍需真实鼠标操作确认事件监视与窗口测量配合。
- 布局/焦点：顶部 UI QA 覆盖统一外圆角、安全区、展开/收起、键盘 Escape、完成组、窄屏和多屏尺寸；模拟数据不改真实任务。
- 可见性：隔离 native smoke 使用生产 Surface/platform 与真实 WebView，验证停靠、悬浮、回吸窗口始终可见，实际 AppKit frame 顶边等于 screen.frame 顶边。
- 物理刘海设备：本机仅有无刘海外屏，报告参数及浏览器模拟不能代替实机验收；不声明硬件验收完成。

## 验证记录

原生 Surface 14 项、前端定向 39 项通过。前端 API 9 构建成功。隔离 native smoke 输出 `Native smoke: Ok(())`：实际 frame `x=852, y=-1080, width=215, height=26`，屏幕顶边 `-1080`。后续实际应用验收及交付版本见本轮完成记录。

### 本轮交付（2026-11-10）

- 顶部 UI QA 通过，四档工作区展开尺寸为 371×293、440×340、587×440、640×440；覆盖安全区 32px、移出刘海后清零、统一外圆角、完成组、焦点与固定顶栏。11 项 webcontent 检查通过，`git diff --check` 无错误。
- 完整 App 版本 `0.3.10-beta+topedge.20261110.1` 构建成功；ad-hoc 签名完整性、16 个前端资源摘要及 API 9 校验通过。退出旧进程后干净替换 `/Applications/Code Pet.app`，确认当前进程来自该路径。旧安装包仅备份为仓库 artifacts 下的 ZIP，没有创建 Downloads 可运行副本。
- 实际 App 验证收起、展开、真实任务列表及通过按钮切换悬浮/顶部，截图保存于 `artifacts/top-edge-release/native-{collapsed,expanded,floating}.png`。外接屏的原生窗口顶边由 smoke 验证。
- 原生浮层可截图和执行 AX 按钮动作，但本轮 CUA 坐标拖动返回 `noWindowsAvailable`；因此不把单元测试与 UI 模拟称为真实鼠标拖动验收。物理刘海及真实鼠标沿顶部拖动/拖出/回吸仍需实机复查。

### 拖动与展开时序（2026-11-10）

用户反馈展开面板开始拖动时先收起，导致抓取位置跳变。源码中 `presentation.ts` 在 dragging 改变时清空 expanded；原生 `begin_drag` 和同模式松手也无条件递增布局 revision，导致有效窗口短暂隐藏并等待重测。

修复：同一 docked 模式的拖动保留 expanded，按下头像或正在拖动时冻结展开/收起决策；进入悬浮或重新吸附仍重置为收起态。hover 进入只申请 200ms 延时，离开、按下头像、拖动或模式切换会取消；延时前开始拖动，直到松手仍保持收起，下一次重新移入才重新计时。直接点击和键盘聚焦仍立即展开。离开延迟维持 240ms。

`NotchPet.svelte` 使用 240ms 自上向下揭示配合正文淡入，不对布局高度插值，避免 ResizeObserver 与原生窗口尺寸相互触发；系统减少动态效果时禁用动画。原生仅在模式、显示器或刘海安全区变化时使布局失效，同屏横向拖动开始和结束沿用有效尺寸。

回归：`presentation.test.ts` 覆盖待展开取消、已展开拖动冻结、收起拖动不展开；`pet::surface` 覆盖两种尺寸下的同屏横向拖动可见性。UI QA 通过真实组件验证短暂停留、提前按下拖动、展开拖动离开区域时不收起、松手、后台悬停和减少动态效果。自动操作工具的原生坐标拖动限制仍存在，UI 模拟不代表真实鼠标链验收。

结果：前端定向 11 项、原生 Surface 15 项、完整顶部 UI QA、native smoke 通过；完整构建为 `0.3.10-beta+topedge.20261110.3`，安装到 Applications。日志 `/tmp/codepet-hover-{tests,native-tests,ui,native-smoke,app-build}.log`。native smoke 首次因 Tauri 构建临时加入的全局 feature 与测试配置不匹配而退出，恢复 Cargo.toml 原有 feature 后重跑通过。

### 整条顶栏拖动与闪烁修复（2026-11-10）

证据：用户截图中动画过程的头像被裁掉下半部分。`.3` 把裁剪动画挂在整个 `.notch-panel` 上，顶部内容受其影响；Surface 测量另有 60ms debounce，展开后的 DOM 会短暂留在旧原生 frame 内。原生 `native_hit` 还仅命中角色框，无法拖动状态文字和顶栏空白。

`NotchPet.svelte` 增加实际顶栏拖动区域，涵盖标题、状态、操作按钮及四周 padding；移动满 5px 才开始事务，拖动后的 click 在捕获阶段拦截，单击和键盘操作仍保留。桌面模式继续使用角色拖动，不把任务列表变成拖动区。

`surface.ts` 测量 `dragRegion` 并与角色 anchor 分开传输；原生按当前屏幕单位转换后命中，不在 Rust 写死前端 header 高度。范围必须有限且在窗口内。角色 anchor 仍服务拖出悬浮。布局测量改为同一渲染轮次合并、下一轮立即发出，不再延迟 60ms。

展开动画仅作用于独立背景层和正文；顶栏祖先不再裁剪或淡入。状态节点在两种模式间复用，收起时仅隐藏附加状态文字。顶栏内边距移到可测量的拖动容器，视觉间距和任务样式不变。

风险与验证：原生测试覆盖负坐标屏幕的左/中/右拖动区域、任务区排除、桌面回退和非法矩形；前端测试覆盖角色锚点独立性与及时尺寸同步。UI QA 验证标题、状态、按钮和留白拖动不触发点击，展开/收起节点不重建、顶栏无裁剪/淡入、减少动态效果及既有列表布局。真实鼠标链仍受本机自动操作工具限制。

结果：13 项前端定向测试、16 项原生 Surface 测试、顶部 UI QA、11 项 webcontent 测试、native smoke 和签名完整性检查通过。完整构建 `0.3.10-beta+topedge.20261110.4` 使用 API 10，替换 Applications 旧安装。webcontent 首次测试遇到 macOS `/var` 与 `/private/var` 临时路径别名断言，使用 `TMPDIR=/private/tmp` 重跑通过；没有为该已有测试路径问题修改业务代码。


### 共享内容展开动效与逐帧验收（2026-11-10）

证据：`.4` 展开时先切换 DOM 布局，再由原生扩大并重新居中窗口。顶栏在新窗口中的局部位置立即改变，旧的背景揭示动画没有补偿屏幕坐标，产生“先跳左，再展开”。引入提交未确认。逐帧复查还发现用 viewport 媒体查询测量目标列表，会让旧窄窗口中的列表先换行，扩容后再缩短；截图中出现了内容先于黑色背景溢出的情况。

修复涉及以下模块：

- `surface/mod.rs` 的 `pet_surface_plan_layout` 复用真实 dock_frame、显示器边界及安全区计算，只预览不写窗口；API 11 防止前端动效与旧后端错配。
- `surface.ts` 在动效事务中暂停 ResizeObserver 自动布局，允许显式提交原生尺寸，结束后恢复测量。
- `notchMotion.ts` 负责具体顶部视图：记录当前屏幕相对位置，保留不可交互的视觉快照等待 native 扩容，随后对共享头像、标题、状态点进行 260ms 平移；字体和图像不缩放。准备期间只将真实内容透明化、保留其焦点与命中；不能用 visibility:hidden，否则会触发 blur/leave 并自动收起。背景独立插值宽高，前景裁剪跟随可见背景，正文淡入，新增状态在尾段淡入避免与移动头像重叠。收起先完成动效，再缩小 native frame；CSS 随 viewport 自动消除位移补偿。反向切换从当前可见帧接续，减少动态效果时即时完成。
- `NotchPet.svelte` 的列表换行改用容器查询，目标布局不受旧 native viewport 宽度影响；外圆角跟随独立背景层。
- `PetApp.svelte` 的鼠标透传/后台 hover 使用动画中的背景矩形，临时扩大的透明画布不成为命中区；切换模式取消当前动画。数据层、任务列表及角色导入继续复用原模块。

验证：19 项前端定向测试、16 项原生 Surface 测试、完整顶部 UI QA、11 项 webcontent 检查通过。`scripts/pet_motion_ui_qa.mjs` 对居中/贴左/贴右的展开与收起、100ms 反向、准备阶段快速反向逐帧记录屏幕坐标、头像尺寸与连续 PNG。逐张检查截图并修复了背景外溢；`scripts/pet_motion_images_qa.py` 另外检测 312 张截图的橘猫像素位置，验证屏幕轨迹并生成对比图/查看器；报告及逐帧查看器位于 `artifacts/pet-motion-qa/`。截图为真实组件和隔离示例数据，不冒充生产任务或 macOS 屏幕录制。

原生 `pet_surface_qa --motion` 使用真实 AppKit frame 与 WebView rAF，覆盖展开、收起和反向；最终采样 129 帧，窗口/viewport 宽度一致、头像不缩放、位移连续。读取 native 后再读取 DOM，避免把新窗口与旧 DOM 坐标错配。浏览器截图由两个测试像素编码当帧 viewport，用于准确对齐 compositor 图片，不能用相邻 rAF 时间戳猜测窗口尺寸。

回归命令：`CODEPET_QA_NODE_MODULES=/tmp/codepet-pet-qa/node_modules node scripts/pet_motion_ui_qa.mjs`、`uv run --with pillow python scripts/pet_motion_images_qa.py`；先 `npm run build:webcontent`，再以 `TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' CODEPET_MOTION_REPORT=<绝对路径> cargo run --manifest-path src-tauri/Cargo.toml --example pet_surface_qa --features custom-protocol -- --motion` 验证原生。报告中的位移阈值结合帧间耗时，丢帧和首尾原生尺寸切换需另外检查截图。

风险边界：真实物理刘海、混合 DPI 跨屏与原生鼠标拖动仍需对应硬件/输入链验收；当前外接屏的原生坐标采样与浏览器拖动回归不能替代这些检查。

交付：最终完整版本 `0.3.10-beta+topedge.20261110.5` 已干净替换 `/Applications/Code Pet.app` 并启动，签名完整性校验通过。CUA 验证实际展开稳定保持、收起正常；真实任务截图为 `artifacts/top-edge-release/native-morph-v5-{expanded,collapsed}.png`。旧 `.4` 只保留仓库 ZIP 备份。


### 原生呈现与浏览器结果不一致（2026-11-10）

用户反馈 `.5` 仍闪烁。此前浏览器像素截图与 native DOM/rAF 坐标采样只能说明各自结果，不能证明 macOS 窗口合成连续。真实 CUA 连续截图复现了空白帧、头像被顶栏动态 clip 裁切和最终缩窗左移；验收结论以本节及 [原生动效排查](../../40-runbooks/pet-native-motion-flicker.md) 为准。

API 12 的 `pet_surface_begin_frame/end_frame` 在 macOS 使用 WKSnapshot 与独立的无边框快照窗口保持换尺寸时的上一帧，目标布局经过 afterScreenUpdates 快照回调后才恢复实时 WebView。前端动画暂停在初始帧，等原生交接结束再播放；收起的末尾缩窗也使用相同握手。快速反向由 token 失效机制及准备阶段请求合并处理，异常恢复 alpha 与观察器，平台逻辑只放在 `platform/pet_surface/macos_frame.rs`。非 macOS 的同一接口直接完成。

顶栏移除动态裁剪，头像、标题及背景统一使用 layout/paint 的 left/top 时序，不混用硬件合成 transform 和布局宽高动画。正文仍可单独淡入。原生尺寸和画面交接使用 NSAnimationContext 事务。列表样式和状态机不变。


验证边界补充：最终原生坐标 129 帧、浏览器与 Surface 回归通过；原窗口级截图无法包含独立快照层，系统桌面录制未能通过当前自动操作启动，所以本轮不声明完整桌面逐帧验收通过。中间版 161 帧没有报错的结果已被后续反向用例推翻，不能作为最终结论。

最终安装：`0.3.10-beta+topedge.20261110.7` 已干净替换 `/Applications/Code Pet.app`，旧进程退出，新进程路径确认。ad-hoc 签名完整性通过；实际展开/收起可交互，展开静态截图位于 `artifacts/top-edge-release/native-morph-v7-expanded.png`。`.5`/`.6` 仅留仓库 ZIP 备份。最终完整顶部 UI QA（含 Escape 后不重新展开）、19 项前端测试、16 项 Rust 测试、11 项 webcontent 测试、浏览器逐帧动效及原生几何回归通过；桌面最终合成逐帧验收仍未完成，不把静态截图当动效证据。
