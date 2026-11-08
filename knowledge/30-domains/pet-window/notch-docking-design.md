# 刘海驻留、拖出悬浮与角色表现接入设计

## 背景

2026-11-08：用户要求研究 Ping Island 的刘海/悬浮切换和角色能力，以及 Notchi 的架构，对照 CodePet 提出优化路径。本文包含源码调研、设计和后续实施记录；下述设计并非全部同时交付，首版已实现范围见文末。用户随后要求拆分顶部状态条/下拉面板与桌面 UI，最新实现及 API 6 交付见[两套 UI 与共享状态](two-views-and-shared-state.md)。

取证基线：CodePet HEAD 444d9f7；Ping Island [6c9bdfeb](https://github.com/erha19/ping-island/tree/6c9bdfebf5324f033124b13a6da762c9a9df5c71)；Notchi [e6f8b8a8](https://github.com/sk-ruban/notchi/tree/e6f8b8a805cec31b5d3844d2ea8faa7a836ebc7e)。工作区已有 Provider 生命周期相关改动，本研究保留它们。

## 目标

- 同一角色可驻留刘海、拖出悬浮、拖回吸附；切换不丢任务、选中项和已关闭气泡状态。
- 保持内容自适应窗口、透明命中、跨屏可见和现有图片/动画包导入。
- 分离任务事实、角色表现、窗口布局和平台操作，使状态与几何可独立测试。

## 非目标

本期不迁移 React/SwiftUI、不替换 Provider 采集链路、不增加模型服务；不把第三方审批 UI 当作已有控制能力，也不直接移植第三方代码或素材。多会话多角色队列、任意图集格式是后续扩展。最初未规划的无刘海顶部回退已在实施阶段纳入，见文末。

## 现状理解

### 上游源码证据

| 证据 | 已确认行为 | 对 CodePet 的启示 |
| --- | --- | --- |
| Ping [IslandPresentationCoordinator](https://github.com/erha19/ping-island/blob/6c9bdfebf5324f033124b13a6da762c9a9df5c71/PingIsland/App/IslandPresentationCoordinator.swift#L37) | 协调器共享 SessionMonitor/ViewModel，切换时关闭旧窗口、建立另一种窗口；保存悬浮锚点 | 模式切换与业务状态生命周期分离；不必照搬两套原生窗口 |
| Ping [IslandPresentation](https://github.com/erha19/ping-island/blob/6c9bdfebf5324f033124b13a6da762c9a9df5c71/PingIsland/Core/IslandPresentation.swift#L56) | 拖出门槛为长按 0.35 秒、向下至少 20pt 且纵向位移大于横向；浮窗内部拖动另有 3pt 门槛 | 点击、拖动、展开必须有明确互斥状态；数值是参考，不是 CodePet 已采用参数 |
| Ping [DetachedIslandWindowController](https://github.com/erha19/ping-island/blob/6c9bdfebf5324f033124b13a6da762c9a9df5c71/PingIsland/UI/Window/DetachedIslandWindowController.swift#L802) | 内容变高时保留角色屏幕锚点；拖动时收起气泡；松手时检查角色锚点是否进入刘海区域，调用 redock；位置保存为可见区比例 | 以角色而非整个窗口判断吸附，避免气泡高度干扰拖动 |
| Ping [ScreenNotchMetrics](https://github.com/erha19/ping-island/blob/6c9bdfebf5324f033124b13a6da762c9a9df5c71/PingIsland/Core/ScreenNotchMetrics.swift#L22) | NSScreen 的 safeAreaInsets 和左右 auxiliary area 推导刘海尺寸，缺失时使用回退值 | 需要原生屏幕能力，不能只靠 CSS 或前端 workArea |
| Ping [MascotSettingsView](https://github.com/erha19/ping-island/blob/6c9bdfebf5324f033124b13a6da762c9a9df5c71/PingIsland/UI/Views/MascotSettingsView.swift#L204) | Picker 枚举 MascotKind，按客户端保存内置角色覆盖 | 上一轮“自定义角色”描述应收窄为内置形象映射；这里不是任意图片/动画包导入 |
| Notchi [AgentProviderAdapter](https://github.com/sk-ruban/notchi/blob/e6f8b8a805cec31b5d3844d2ea8faa7a836ebc7e/notchi/notchi/Services/AgentProviderAdapter.swift#L11)、[SessionStore](https://github.com/sk-ruban/notchi/blob/e6f8b8a805cec31b5d3844d2ea8faa7a836ebc7e/notchi/notchi/Services/SessionStore.swift#L10) | 适配器 normalize 原始事件；以 ProviderSessionKey 管理各会话，支持显式选择和默认选择 | 统一事件、会话身份和角色选择策略，避免组件识别具体 Agent 原始字段 |
| Notchi [NotchiState](https://github.com/sk-ruban/notchi/blob/e6f8b8a805cec31b5d3844d2ea8faa7a836ebc7e/notchi/notchi/Models/NotchiState.swift#L138)、[StateMachine](https://github.com/sk-ruban/notchi/blob/e6f8b8a805cec31b5d3844d2ea8faa7a836ebc7e/notchi/notchi/Services/NotchiStateMachine.swift#L54) | task、emotion、spriteFamily 分开；状态协调器还管理文件监听、声音、情绪请求和存活监测 | 借鉴分维度状态，但它不是纯 reducer；CodePet 应将 IO 与可重放的状态转换进一步分离 |

Ping 的刘海窗口实际是屏幕全宽、750pt 高的透明窗口，见 [NotchWindowController](https://github.com/erha19/ping-island/blob/6c9bdfebf5324f033124b13a6da762c9a9df5c71/PingIsland/UI/Window/NotchWindowController.swift#L43)。其 [NotchPanel](https://github.com/erha19/ping-island/blob/6c9bdfebf5324f033124b13a6da762c9a9df5c71/PingIsland/UI/Window/NotchWindow.swift#L41) 使用高于菜单栏的层级、鼠标路由和事件重投。CodePet 不照搬大透明 frame 或事件重投，继续遵守[内容尺寸](layout-and-sizing.md)与[透明透传规约](../../60-rules/pet-window-click-through.md)。

### 调研时的 CodePet 基础与缺口（实现前）

- [PetApp.svelte](../../../frontend/PetApp.svelte) 每秒取 Pet 快照；同时负责选屏、限位、ResizeObserver、原生拖动和透传。窗口宽 320px、高度随内容，启动总是放到右下角。目前没有刘海模式或位置恢复。
- 调研时的 windowFrame.ts（现已由 [Surface 几何模块](../../../src-tauri/src/pet/surface/geometry.rs) 替代） 固定保留窗口右下角。任务气泡换到下方、角色缩放或刘海展开时，需要改为明确的角色/停靠锚点。
- [macos_window.rs](../../../src-tauri/src/platform/macos_window.rs) 已有非激活 NSPanel、跨 Space 和按需获得键盘焦点；尚无刘海几何、吸附和模式相关窗口层级。
- [projection.rs](../../../crates/codepet-host/src/pet_gateway/projection.rs) 已按来源+会话归并、去重、拒绝旧轮事件，断连标 unknown。[animation.ts](../../../frontend/lib/pet/animation.ts) 则把全部任务压成单个状态：等待优先，失败次之；completed 可持续循环庆祝，unknown/interrupted 落入 idle，thinking 素材尚无对应快照映射。这些是表现策略优化点，不应修改任务事实来实现。
- [packages.rs](../../../src-tauri/src/pet/packages.rs) 已支持 1536×1872、8×9 的 PNG/WebP 动画包；[library.rs](../../../src-tauri/src/pet/library.rs) 保留静态图片、托管导入与目录发现。当前比 Ping 的内置角色选择更符合 GPT 生成素材导入需求。
- [PetGateway](../../../crates/codepet-host/src/pet_gateway/mod.rs) 当前提供观察快照/来源开关，快照 approvals 为空。见[Provider 活动订阅](../../10-architecture/provider-hook-observation-proposal.md)：Pet 控制动作尚未实现，不应绕过协议安装第二套 Hook。

## 实现路径

### 1. 保留现有业务链，拆出表现与窗口协调

推荐的数据流为：Provider observation → Host Pet projection → PetSnapshot → PetPresentationStore → Svelte 角色/气泡；窗口意图和布局测量另送 PetSurfaceCoordinator → 平台窗口适配器。

- PetSnapshot 是任务事实；前端共享 store 管理过滤、选择、关闭记录和角色表现，不随刘海/悬浮切换销毁。
- 首期复用同一个 pet WebView/NSPanel，切换 Svelte 布局和原生窗口策略。先减少窗口交接和快照重复订阅；若实机证明面板焦点无法兼容，再拆两个窗口，共享 store 语义不变。
- 原生协调器成为位置/尺寸写入的唯一入口。前端 ResizeObserver 只上报自然 contentSize、petAnchorInContent、交互区域及 layoutRevision；原生处理模式、显示器、拖动和原子 frame 更新。旧 setSize/setPosition 调用要一起收口，不能两边争抢位置。

### 2. 统一屏幕接口，按模式限位

先定义跨平台 PetSurface 接口，输出 displayId、screenRect、workArea、scaleFactor、notchRect?、能力标志与 geometryRevision。macOS 适配器读取 NSScreen；Windows/Linux 返回无物理刘海能力并继续悬浮，不在业务组件散落平台判断。

前端测量用 CSS 逻辑单位；平台边界负责 AppKit 左下原点与 Tauri 全局坐标、缩放的转换。跨屏使用目标显示器几何，不能对全局坐标简单乘一个 scaleFactor。显示器增减、DPI/工作区变化时使旧几何失效。

- floating：角色及实际气泡限制到工作区，气泡根据上下空间选择展开方向。
- docked：按屏幕顶部的刘海几何锚定，内容从刘海下缘向下展开；物理摄像头区域只作为定位参照，不能放交互控件。
- dragging：暂缓普通工作区限位和内容改帧，允许进入顶部吸附带；松手后才提交 docked 或重新钳制 floating。不能继续无条件调用旧 constrainWindowToScreen，否则到达刘海前就被拉回。

浮窗保存 displayId + 工作区归一化角色锚点；屏幕消失时恢复到可用显示器。停靠保存目标显示器与模式。保留无设置时右下角默认行为，热加载前端先读取原生状态而不是再次右下角初始化。

### 3. 拖动作为显式事务

稳定模式仅 floating/docked；独立 drag 状态保存起点模式、角色抓取偏移、目标显示器、吸附候选和序号。流程：按下 → 超过手势门槛 → 原生跟踪屏幕指针 → 显示吸附预览 → 松手提交；Escape/取消恢复起点，丢失捕获必须清理。

首期沿用易理解的角色拖动，不直接引入 Ping 的长按规则。松手时以角色锚点落入扩大后的刘海吸附区为判据；进入/离开阈值使用滞回，避免边缘闪烁。拖动时收起临时气泡、保持鼠标接收；结束后恢复最新任务和透传状态。原生层可靠接收 mouse-up 的具体方式需先做小规模验证，不能把 startDragging Promise 返回直接等同于全平台拖动结束。

窗口永远按当前内容测量：floating 保持角色锚点，docked 保持刘海顶部中心锚点。动画装饰只加有限余量，不恢复固定最大高度。所有异步布局/显示器结果检查 revision，旧结果不能覆盖拖动后的新位置。

### 4. 角色资产与 Agent 身份解耦

第一阶段继续统一使用当前 PetAvatar、静态图片和 8×9 pet.json。AI 生成来源不影响加载：合规图集走现有包导入；普通单图仍为静态角色，不自动拥有多状态动画。

增加可选 providerPetOverrides，选中会话明确时按其 provider 找角色；没有选中时按“等待交互 → 失败 → 活动 → 空闲”的稳定策略挑会话，角色映射为空回退用户全局选择，删除角色清理悬空引用。默认仍是一只全局宠物，不因新增来源自动改变用户外观。

需要更多布局时再引入带 schemaVersion 的 manifest：帧尺寸、动作帧序列/时长、角色锚点与缺失动作回退。旧包无版本时转换成当前固定描述，不修改原素材；渲染和 alpha 命中共享描述。先统一内部 asset descriptor，再扩展文件格式，避免在包解析、动画和 hitTest 中各写一套尺寸规则。

### 5. 借鉴 Notchi，进一步隔离状态和副作用

- 表现状态分成 taskPhase、attention、gesture、surface；形象是 assetId，避免以 Provider 名决定动画文件。waiting-input 与 waiting-approval 保留语义差异，unknown 显示待确认提示。
- 完成庆祝只在可识别的新终态边沿播放一次，再回 idle；任务卡继续保留完成。断连不能触发庆祝，旧失败记录不能永久抢占新任务。一次性效果须明确标识与消耗机制，不能以每次 snapshot/revision 更新都重播。
- 有准确事件证据后才细分 thinking、toolWorking、compacting；没有证据时保持 running，不按时间猜真实 Agent 状态。新增投影字段需修改 Pet IDL、生成 SDK 与兼容测试。
- 将 projection 中来源字段解析拆成纯 adapter，状态 reducer 只接统一事件；adapter 仍属于 Pet 的消费投影层，Provider 继续发布中立观察事实，不生产 PetTask。保留现有去重、轮次隔离、未知状态与控制/观察通道边界。
- 后续让 Host 的 revision 变化发出“快照失效”通知，UI 合并拉取原子快照；先订阅再初始化、恢复时重取，保留低频兜底，避免遗漏最后状态。无需把整套事件流搬到 Svelte。
- 声音、完成通知、短暂情绪等由效果调度器处理；首期不接 Notchi 的远程 prompt 情绪分析，桌宠运行与导入不依赖 GPT/API Key。

### 交付顺序

1. 提取表现 store、锚点几何和 Surface 接口，先证明普通悬浮行为不变。
2. macOS 原生验证刘海定位/拖动结束/菜单栏透传；实现吸附、拖出、回吸及重启恢复，发布新原生版本。
3. 接 provider 角色映射和表现状态时序；保留全部现有导入入口。
4. 按需要独立推进任意图集、事件通知优化和多角色布局。

## 涉及模块

- frontend/PetApp.svelte、lib/pet/：窗口职责拆出、两种布局、共享表现状态、角色锚点测量与命中。
- src-tauri/src/pet/、platform/：新增统一 Surface 协调及平台几何/窗口/拖动适配，库导入继续使用现有模块。
- src-tauri/src/app/settings.rs、frontend/lib/types.ts、lib/api.ts：保存模式、显示器、位置与可选角色映射，默认值兼容。
- src-tauri/src/lib.rs、capabilities/：注册窄命令与事件；macOS re-export 和 tests 的模块路径同步检查。
- crates/codepet-host/src/pet_gateway/、protocol/pet/v1 及 SDK：只在表现语义和通知优化阶段修改，保留现有任务投影约束。
- webcontent-contract.json：新增原生命令要求 backendApiVersion 从当前 4 升级；不能仅替换前端包获得刘海能力。完成原生升级后，布局和资源迭代继续走独立 webcontent。

## 风险

| 风险 | 验证/缓解 |
| --- | --- |
| 大透明区域再次挡住鼠标或提前触顶 | UI 量测 frame 等于内容+有限余量；真机点击/滚动下方应用与菜单栏 |
| 顶部限位、坐标翻转、跨屏缩放使角色跳动 | 几何单测覆盖负坐标、上下排列、1×/2×混合屏；真机反复跨屏回吸 |
| resize、拖动、显示器事件竞争 | 单写入协调器、revision 丢弃陈旧结果；事件交错测试及拖动期间任务增减验收 |
| 模式切换抢焦点或失去输入焦点 | NSPanel 按需成为 key；验收后台通知不抢焦点、显式打开后键盘操作正常 |
| 角色切换/删除/坏包导致闪图或命中错位 | 现有异步请求序号与回退测试保留；新增映射清理和共享帧描述测试 |
| 跨模式重复庆祝、重复订阅或会话状态混淆 | 同一 store 生命周期；测试多会话、旧轮迟到、断连、关闭记录和切换时效果不重播 |
| 原生与前端契约不匹配 | webcontent 兼容拒绝测试，升级二进制后再安装新前端 |

## 测试计划

实现阶段新增纯 reducer/geometry 测试：拖出/回吸/取消、两类锚点、吸附滞回、屏幕移除、缩放、终态效果只播放一次、来源映射和资源失败。

复用 pet_library_tests、settings_tests、macos_window_tests 与 frontend/lib/pet 下测试；将 PetApp 中与重构冲突的源码字符串断言替换成行为检查。扩展 scripts/pet_window_frame_ui_qa.mjs、pet_module_ui_qa.mjs：两种模式的无任务/单任务/多任务、自定义图集、气泡方向、布局/焦点/按钮可见性及响应尺寸。UI mock 不能替代原生测试。

真机矩阵：有刘海内屏、无刘海外屏、混合 DPI、菜单栏自动隐藏、全屏 Space、睡眠恢复、拔屏；分别检查点击、滚动、拖动结束、快速来回切换及重启/前端热加载恢复。

调研阶段执行 npx --no-install vitest run frontend/lib/pet/animation.test.ts frontend/lib/pet/hitTest.test.ts frontend/lib/pet/PetAvatar.test.ts frontend/PetApp.test.ts：4 个文件、20 项全部通过。git diff --check 通过。未构建或运行两个上游应用，未实测新刘海行为；现有基线通过不构成新功能验收。

## 知识沉淀

本设计位于 pet-window 域。实施后已更新窗口系统、布局、跨屏与透明透传规约；角色导入沿用原文档。本文前半保留调研时方案，交付范围与证据以实施记录为准。

## 未知项

- 外屏顶部停靠、实际拖出/回吸和重启恢复已验证；物理刘海、全屏 Space、菜单栏自动隐藏、睡眠与拔屏仍需设备验收。
- Ping/Notchi 的发布包性能与机器兼容性未实测；代码测试名只说明覆盖意图，不表示本轮执行过。
- 任意 GPT 导出格式不等于当前 8×9 契约；扩展前需拿具体样例验证元数据、锚点、透明边距和帧布局。
- 若后续直接复用上游代码/素材，应分别核对 Ping 的 Apache-2.0、Notchi 的 GPL-3.0 与 CodePet 当前分发许可；本轮独立设计和实现，没有复制上游代码或素材。


## 2026-11-08 实施记录

用户授权后已实现单窗口的 floating/docked 切换、拖出、吸附候选、松手回吸、Escape 取消、屏幕变化恢复和状态持久化。新增 `pet/surface/{mod,geometry}.rs` 与 `platform/pet_surface.rs`，前端使用 `lib/pet/surface.ts`；旧前端窗口改帧和边界代码已删除。

本机当前只连接 LG ULTRAFINE 无刘海外屏，因此补充 macOS 顶部居中回退，并将按钮文案区分为“吸附到刘海”和“吸附到屏幕顶部”。真刘海仍使用系统 safe area。原生鼠标监视器只消费已越过阈值的角色拖动，普通按钮点击与透明命中保持原路径；不注册键盘全局监听，不重投鼠标事件。

图片、pet.json 图集、库目录发现及当前选择未更改；provider 角色映射、多角色、情绪状态重构和快照通知仍为后续工作。新原生命令对应 webcontent API 5。

### 构建隔离

工作区同时发生 Provider/SDK releaseControl 修改，导致原工作区协议测试出现新增方法与预期列表不一致；本任务没有改动或回退这些文件。预览构建在 /tmp/codepet-notch-build-20261108，以已提交 ad5c0f6 为基线，仅复制桌宠相关变更。隔离工作区的 protocol:check 22 项通过。

### 验证

- 隔离工作区 npx --no-install vitest run frontend：217 项通过。
- cargo test --manifest-path src-tauri/Cargo.toml --lib pet::surface：10 项通过。
- pet_library_tests、settings_tests、macos_window_tests：33 项通过，覆盖自定义包/图片与原生 panel 配置。
- TMPDIR=/private/tmp npm run test:webcontent：11 项通过。
- pet_module_ui_qa.mjs 通过：素材、导入入口、透明命中、按钮、动作及 280/320/360px 布局。
- pet_window_frame_ui_qa.mjs 通过：角色锚点、上下布局、任务增减、滚动、焦点、负坐标与缩放，无 resize 反馈循环。
- 原工作区全库 tsc --noEmit 仍有既有 Node 类型、fixture 和 CSS module 诊断；新 surface.ts/test.ts 未产生诊断。

- macOS LG ULTRAFINE 外屏实际运行：按钮停靠、角色拖出成为悬浮、拖回顶部松手恢复停靠均通过；气泡随模式换到角色上方/下方，当前熊猫形象保持不变。退出并从交付目录重启后恢复停靠。
- 初版仅靠 WebView 发起拖动的原生验收未通过；补充 pet panel 局部 NSEvent 拖动监视和 mouse-up 清理后，同一实机手势通过。排查时应检查原生模式/落点，而不能用 UI mock 或按钮成功替代拖动验收。
- codesign --verify --deep --strict 与 hdiutil verify 均通过；包为本机 ad-hoc 签名预览构建。

交付版本 `0.3.9-beta+notch.20261108.2`，macOS arm64。应用和 DMG 位于 `/Users/wangxin/Downloads/codepet-notch-preview-20261108/`。内置 webcontent 的 backendApiVersion 为 5，需升级原生应用；旧 API 4 二进制不能只替换此版本前端包。

DMG SHA-256：`7336a674b4cabfba562dcb14dc69949a2bce1c98a20724b5e15e3ffd8842109e`。

物理刘海屏、混合 DPI 多屏、Windows/Linux 的实际窗口行为尚未验证；对应几何/状态测试不代替这些设备验收。
