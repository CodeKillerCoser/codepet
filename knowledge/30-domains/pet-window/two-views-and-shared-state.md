# 顶部面板与桌面角色的两套 UI

## 背景

2026-11-08，用户指出顶部吸附不应只是完整桌宠与气泡换个位置：收起时应是一行头像和最新状态，hover 后展开规整深色任务面板。用户进一步要求两套具体 UI 独立实现，共用数据与状态机，并明确不要提前引入 UI 接口或多态实现。

## 目标与非目标

顶部使用紧凑状态条与下拉面板；桌面使用完整角色与气泡。模式切换保留角色资产、任务与关闭记录，继续支持拖出、拖回和内容自适应。

本次不增加 UI 适配接口、组件注册器、第二条 Provider 任务订阅或第二个原生窗口。不扩展素材包格式，也不引入头像生成服务。

## 当前实现与模块

- `frontend/lib/pet/model.ts`：两套 UI 共用一个 Svelte store、快照请求与关闭历史。按更新时间排序选出最新状态；断连时将活动任务标为 unknown；拒绝组件销毁后的迟到响应。每秒刷新仍由宿主启动，视图不自行请求 Gateway。宿主另读取并监听已有 Provider identity 信息（不触发运行时探测），统一将 `dev.codepet.*` 标识与 Pet source 短 ID 对齐，供图标与名称展示。
- `frontend/lib/pet/presentation.ts`：纯状态转换管理展开、hover、键盘焦点、桌面任务收起；接收原生 Surface 状态，原生仍是吸附/悬浮/拖动事务的权威。切换模式清理临时展开，不清空任务。
- `NotchPet.svelte`：收起条按内容宽度自适应，含装饰边距最多 280px，高 44px；头像、状态点、最新任务标题和数量同排。展开宽最多 640px，顶部当前活动栏与列表同宽、共用容器，只有下方任务区域滚动。两种宽度都受原生 availableWidth 限制。hover 或键盘进入展开，离开 240ms 后收起，重新进入取消收起，键盘焦点在面板内时保持展开。Escape 收起并将焦点移回状态条。
- `DesktopPet.svelte`：独立保留完整角色、分组气泡、操作栏和鞭子交互；任务折叠与分组展开状态由宿主保存，切换 UI 后可恢复。
- `PetApp.svelte`：装配两套具体组件，处理设置、共享订阅、计时器与原生命令，不再包含两套视图的具体布局。
- `PetAvatar.svelte`：复用相同资源加载器与动画帧，portrait 将完整帧缩放到 48px 宽，在 48×32px 视口内显示上部，取消旧的左右裁切和负向顶部偏移，给耳朵与脸部留空间。静态图、8×9 图集和错误回退沿用现有能力；没有新 UI 接口。
- `pet/surface/geometry.rs`：停靠窗口以整个面板居中，头像锚点仍负责拖动抓取与浮窗尺寸变化。否则左侧头像会把整个顶部面板推偏。

顶部列表对标 Ping Island：左侧真实 Provider 图标，右侧单行标题、Agent 名称胶囊与最多两行消息。没有运行时长数据，不显示运行时长；时间取 `updatedAt`，作为最近更新时间（HH:mm）展示，并提供完整日期提示；运行状态独立显示。移除分隔线、进度条、列表标题和底部栏。悬浮/主窗口入口收进固定顶栏的图标按钮。图标缺失或加载失败时使用 Plug 图标，不杜撰品牌素材。顶部两侧使用 16px 外侧圆角，原生 frame 为装饰各留 16px 边距。它不沿用桌面气泡的颜色、自定义透明度和叠层，固定使用深色主题 token 与黑色背景。真实任务区域内部滚动，收起不保留透明占位。

原生面板居中语义与旧 API 5 的角色居中不同，当前 webcontent 契约升为 6，防止新 UI 被装进旧预览二进制后整体偏移；这次需更新完整应用。

## 风险与验证

- 展开后鼠标落点丢失：展开顶栏与列表同宽，整个面板属于同一 hover 区域，浏览器验证从条移入列表、短暂离开再进入与延迟收起；外屏原生验收也通过。
- 键盘焦点被自动收起吞掉：focus-visible 进入保持面板，Escape 回到状态条；浏览器执行 Tab、Enter、Escape 和顶栏按钮聚焦。
- UI 切换造成重复拉取、关闭记录丢失：model 单测覆盖请求合并、迟到响应、关闭记录恢复；UI 验证顶部关闭任务后切回桌面仍隐藏。
- 头像拖出后抓取偏移、窗口中心错误：几何单测覆盖非居中头像与面板居中；UI 模拟拖出/松手回吸，实际 macOS 手势另验收。
- 自定义角色头部位置不统一：头像当前是固定上部裁切，不是语义识别人脸。标准内置图集已目视检查；任意静态图片可能裁切不理想，后续有明确样例再考虑可调裁切，不提前扩展包格式。

## 验证记录

- `npx --no-install vitest run frontend`：37 文件、224 项通过。
- `TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo test --manifest-path src-tauri/Cargo.toml --lib pet::surface`：10 项通过。
- `CODEPET_QA_NODE_MODULES=/tmp/codepet-pet-qa/node_modules node scripts/pet_notch_ui_qa.mjs`：收起/hover、离开延迟、键盘/Escape、共享关闭记录、导入图集、模拟拖出回吸、Provider 插件 ID 映射、收起最多 280px / 展开 640px、窄屏滚动与顶栏固定全部通过。截图位于 `artifacts/pet-notch-qa/`。
- 同环境 `pet_module_ui_qa.mjs`、`pet_window_frame_ui_qa.mjs` 通过，桌面无任务 126px、单任务 243px，角色锚点和透传回归保留。

## 知识沉淀与未知项

窗口系统、布局和透明命中规约同步更新。浏览器 IPC 模拟不等于原生窗口验收；物理刘海、混合 DPI、多平台与任意图片头部裁切尚不能由本机外屏测试覆盖。

## 前次原生验收与交付（.3 版本）

macOS LG ULTRAFINE 外屏运行通过：44px 状态条、鼠标进入展开/移出收起、Tab 进入、Escape 收起回焦、头像拖出变完整角色、拖回变状态条，均通过真实窗口操作验证。内置熊猫与当前任务在两种 UI 间保持一致。从交付目录重启后恢复顶部收起状态。原生截图 `native-collapsed.png`、`native-expanded.png`、`native-desktop.png` 已保存。

完整构建基于当前工作区已提交基线 b60ad7c 与本任务改动（前次 Provider/SDK 工作已提交，此次无需隔离旧基线）。`protocol:check` 22 项、`test:webcontent` 11 项通过；完整 app/DMG 构建、内置 16 个文件哈希检查、codesign 与 hdiutil verify 全部通过。

交付版本 `0.3.9-beta+notch.20261108.3`，macOS arm64，API 6，本机 ad-hoc 签名预览版。交付目录 `/Users/wangxin/Downloads/codepet-island-preview-20261108/` 包含应用、DMG 与三张实机截图。

DMG SHA-256：`5566a77de0d407c61a8a5aeb088117b282606256d182ff9223ce7e7555418ad2`。


## 任务行与固定顶栏修订（.4 版本）

依据用户 Ping Island 截图与后续澄清完成：Provider 图标、标题/Agent 名称同排、最多两行消息；移除运行时间、进度条、列表标题与底栏。展开顶栏和列表共享同宽容器且固定不滚动；收起才缩为内容宽度。顶部外侧圆角与熊猫完整耳朵/脸部通过原生截图确认。真实手势拖出、拖回均通过，最终停留顶部收起状态。

`0.3.9-beta+notch.20261108.4` 已构建为完整 arm64 app/DMG，API 6 未变。交付目录 `/Users/wangxin/Downloads/codepet-island-ui-preview-20261108/`，包含三张原生截图。224 项前端测试、三组 UI QA、22 项协议检查、11 项 webcontent 检查通过；签名与内置 16 个资源哈希已验证。新版未修改 Rust Surface 行为，不重复扩大原生测试范围。视觉修订与对照记录见仓库根目录 `design-qa.md`。


## 时间、状态与间距修订（.5 版本）

用户要求恢复桌面气泡已有的时间和状态。证据：`PetTask` 只有 `updatedAt`，桌面 `PetActivityGroups.svelte` 将其格式化为 HH:mm，并无运行开始时间。本次顶部任务行沿用相同语义，显示“最近更新”时间及独立状态；展开顶栏也显示最新任务状态。

`pet_gateway/projection.rs` 在收到活动但缺少 title/thread_name/prompt 时使用“正在处理任务”作为标题；旧顶部 UI 在 summary 为空且 running 时显示“正在思考中”。二者均为缺少字段时的兜底，不代表未收到任何任务事件，也不是实际推理状态。本次正文为空统一提示“尚未收到回复内容”，不再用状态代替回复。后端标题规则不变。

`NotchPet.svelte` 标题行最小高度从 28px 缩至 24px，消息上边距从 5px 缩至 1px、行高从 1.65 缩至 1.5；相邻任务额外间距从 6px 增至 12px。保留固定顶栏和两行消息。`pet_notch_ui_qa.mjs` 覆盖三种状态、时间 ISO 值/语义标签、缺失正文提示及窄屏布局；34 项相关前端测试通过。没有更改协议或原生窗口行为。

交付版本 `0.3.9-beta+notch.20261108.5` 位于 `/Users/wangxin/Downloads/codepet-island-ui-preview-20261108-5/`，已启动。原生展开截图确认独立的“进行中”、HH:mm 与“尚未收到回复内容”；另保存三条测试任务的布局截图。完整构建、22 项协议检查、签名/16 个资源哈希/DMG 校验通过。实机验收过程中也观察到旧版在收到下一次用户请求后能显示真实标题，进一步确认默认标题是缺少标题字段时的兜底，而非永久固定文案。

用户复查认为任务间距仍偏大。后续收紧为每行上下 8px，取消相邻行额外 margin，任务之间的 CSS 留白由 36px 降至 16px；标题与正文 1px 间距保持。顶部 UI QA（包含固定顶栏、窄屏、滚动、关闭和拖出回吸模拟）通过。本次仅构建前端包并安装至实际数据目录 `versions/webcontent/v2`，重启 .5 应用加载；未重新打包 DMG。


## 旧实现清理验收（2026-11-08）

按入口与引用链核对：`PetApp` 已仅装配共享 `model` / `presentation` / `surface` 和两个具体 UI。旧 `activity`、`runtimeGatewayActivity`、`runtimeGateway`、`agentInteractions`、`codexDesktopCompanion` 前端模块没有生产入口，仅由各自旧测试或未调用的通知判断函数相互引用，因此删除整条死代码链及对应测试。分组测试改为直接验证当前 `PetTask`。旧 `windowFrame`、像素颜色命中辅助函数、无调用的回复/审批/激活封装、旧按钮/编辑器样式和主题变量也已移除；QA 不再模拟旧窗口改帧接口，Tauri 去掉不再使用的光标位置与尺寸写入权限。

保留边界：主窗口事件记录仍使用 `PetEvent`，声音试听仍使用 `playNotificationSound`，桌面分组气泡与自定义图片/图集导入仍有入口。后端 Agent 控制、collector 和生成的协议 SDK 是其他功能的共享边界，本次没有删除。文档中旧回复编辑器、前端事件投影与重复通知已明确标为历史，不再宣称当前桌宠支持这些操作。

清理回归结果：

- `npx --no-install vitest run frontend`：32 文件、166 项通过。数量减少来自删除废弃模块的测试；当前 model、presentation、surface、头像和分组测试保留。
- `TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo test --manifest-path src-tauri/Cargo.toml --lib pet::surface`：10 项通过；同配置 `--test pet_library_tests --test macos_window_tests`：17 + 3 项通过。
- `CODEPET_QA_NODE_MODULES=/tmp/codepet-pet-qa/node_modules node scripts/pet_notch_ui_qa.mjs`、`pet_module_ui_qa.mjs`、`pet_window_frame_ui_qa.mjs` 通过。`pet_groups_ui_qa.mjs` 默认 Edge 在本机不可用，使用同环境加 `CODEPET_QA_BROWSER=chrome` 后通过，覆盖分组、焦点与 280/320/360px 布局。
- `npm run build:webcontent`、`npm run protocol:check`（22 项）、`TMPDIR=/private/tmp npm run test:webcontent`（11 项）通过。构建仍有既存 `ProviderRuntimeInfo.svelte` 未使用 CSS 选择器提示，与本次桌宠清理无关。
- 旧模块与函数引用检索未发现生产消费者；保留的 `PetEvent.shouldRing` 是主窗口事件协议字段，不是旧通知函数。

清理没有改变已验收的面板布局和原生拖动逻辑；物理刘海、混合 DPI 与其他平台的实机覆盖仍沿用上面的未知项。本次生成前端构建用于验证，没有再次制作或安装完整应用。
