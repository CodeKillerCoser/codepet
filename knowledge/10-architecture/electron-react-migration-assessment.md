# Electron + React 迁移成本评估

状态：2026-11-24，用户决定暂不迁移，继续使用 Tauri + Svelte + Rust；开始评估 Svelte 组件库，见 [组件库评估](../50-decisions/svelte-component-library-assessment.md)。以下为基于 `e89d0baf` 的静态评估，未创建 Electron 原型。工期是工程估算，不是实测结果。

## 背景与目标

评估从当前 Tauri 2 + Svelte 5 + TypeScript 迁至 Electron + React，业务继续使用 Rust 的成本。覆盖现有主窗口、桌宠、Provider、远程连接、资源独立更新和正式分发。Mantine 可作为主窗口组件库，但不是采用 React 或 Electron 的前提。

本轮不修改应用实现，不新增桌面对话功能，不改变 Provider/Gateway 协议，不重做视觉设计。历史提案不视为当前实现。

## 证据与现状

核对 `package.json`、`src-tauri/Cargo.toml`、`vite.config.ts`、命令实现、窗口适配器及附近测试：

- `frontend/` 有 32 个正式 Svelte 组件、5,837 行，另有 3 个 QA 组件；最大文件 `App.svelte` 为 2,350 行。行数包括样式与空行，只用于说明规模。
- 前端非测试 TypeScript 有 48 个文件、2,787 行，测试有 39 个文件、2,732 行。组件或非测试 TS 中有 24 个文件直接引用 `@tauri-apps/`，其中包含 QA 入口。迁移不能只替换 `api.ts`。
- `navigation.ts`、`webcontent.ts`、`pet/model.ts` 等还使用 Svelte store；部分状态转换、几何、动画、Markdown、协议类型可继续复用，生命周期和订阅需要适配 React。
- `src-tauri/src/` 有 60 个 Rust 文件、22,572 行；其中 22 个文件出现 `tauri`，这些文件合计 10,830 行。这不等于需要重写 10,830 行。静态匹配到 71 个 `#[tauri::command]` 声明，不代表 71 个独立业务服务或全部均由 UI 使用。
- `crates/codepet-host/src/` 有 55 个 Rust 文件、23,532 行。扫描 `crates/` 与 `sdk/rust/`，`tauri` 仅出现于两个 Provider 边界测试，生产源码未发现引用；结合 Cargo 清单，Host/Provider/SDK 具备保留基础。
- `runtime_gateway/tauri_bridge.rs` 的 `from_app`、事件桥及 `lib.rs` 的 setup 仍负责目录、运行时、后台任务和退出装配。独立 crate 不等于已经有完整可独立启动的桌面业务守护进程。

源码证据入口：`frontend/lib/api.ts`、`codepetGateway.ts`、`pet/surface.ts`、`pet/model.ts`；`src-tauri/src/lib.rs`、`runtime_gateway/tauri_bridge.rs`、`pet/surface/mod.rs`、`platform/pet_surface.rs`、`platform/pet_surface/macos_frame.rs`、`platform/macos_window.rs`、`platform/window_drawer.rs`。相关既有知识见 [窗口系统](window-system.md)、[资源分发](webcontent-packaging.md)、[原生闪烁排查](../40-runbooks/pet-native-motion-flicker.md)。较早架构文档的业务数据流描述以当前源码为准。

## 结论与备选方案

Rust 业务层可以保留。工作量主要是 Svelte UI 改写、Tauri 应用装配剥离、桌面 API/事件桥、原生窗口交互和发布链迁移。React 和桌面容器可以独立选择。

以下按一名熟悉本仓库、Rust、React 和桌面开发的全职工程师估算，1 人日约 8 小时；包含开发、集成与回归，按当前功能对齐，不含新功能和大幅改版。范围之间不能相加。

| 路线 | 估算 | 主要价值与成本 |
| --- | --- | --- |
| Tauri + React，保留 Rust | 12–22 人日 | 获得 React/Mantine 生态；改组件、store、入口及 UI 测试，保留现有原生窗口和发布链 |
| Electron + Svelte，保留 Rust | 25–45 人日 | 验证 Chromium/Node 容器收益；保留大部分 UI，仍需换 IPC、窗口和分发 |
| Electron + React，保留 Rust | 40–70 人日，约 8–14 个工作周 | 同时切换 UI 与容器，覆盖 macOS/Windows 现有正式分发行为；Linux 若要求新增正式发布验收，另估 |

完整路线的估算拆分：风险原型 3–5 人日；Rust 服务抽离及本地传输 6–10；React UI/状态适配 10–16；桌宠原生窗口与抽屉 8–16；资源、安装与升级链 5–9；联调和跨平台回归 8–14。合计 40–70 人日。已计入常规回归，原型若揭示窗口合成或权限障碍，需要重估上限。两人可并行 UI 与容器，但不能直接把日历工期减半。

## 迁移必要性与收益判断

依据 [产品意图](../00-project/product-intent.md)，当前产品核心是远程控制与 Provider 插件平台，桌宠是附加体验，主窗口承担配置管理。结合现有实现，尚无必须整体切换 Electron + React 的证据。推荐保留现有栈，把 React 和容器更换作为两项独立决策；组件生态需求尚未落实到具体受阻功能时，也不建议立即重写全部 Svelte。

| 候选收益 | 能获得什么 | 对当前项目的判断及验证 |
| --- | --- | --- |
| 统一渲染引擎 | Electron 统一采用 Chromium，减少不同系统 WebView 引擎带来的差异；仍有 OS/GPU 差异 | 有潜在价值。先整理确属 WebView 的缺陷与维护耗时，并用同一个页面对照复现；现有原生 resize 闪烁不能直接当成收益证据 |
| Node 集成 | Electron main 直接使用 Node API 和相关模块 | 当前业务主力是 Rust，Provider 已独立运行；在没有具体 Node-only 集成诉求前，直接收益较小。以一个目标依赖完成能力及维护成本比较 |
| React/Mantine 组件 | 复用 React 组件、交互方案与团队经验，未来复杂桌面工作台可能受益 | 潜在收益比换容器更直接。选择实际要开发的复杂页面，对照现有 Svelte 的实现成本；[Tauri 前端配置](https://v2.tauri.app/start/frontend/)允许独立选择前端，不需同时迁 Electron |
| 调试与自动化 | Chromium DevTools、Tracing 和 Electron 进程工具 | 可减少特定引擎的排障摩擦，但现有项目已有浏览器 QA 和原生逐帧验证。用真实故障复现与定位时间判断增益，原生合成仍须实机验证 |
| 性能、稳定性与视觉质量 | 可能改善特定 WebView 问题；UI 库能减少通用控件实现 | 没有证据表明整体更快、更省电或自动更稳定。Rust、网络和 Provider 故障不会因换 UI 容器自然解决；需测实际安装包与具体缺陷 |
| 团队协作 | 若团队与后续人员主要熟悉 React/Node，可降低后续开发门槛 | 当前团队结构与招聘需求未知。应结合实际长期维护人员判断，不能用生态热度替代收益证据 |

Electron 的 [进程模型](https://www.electronjs.org/docs/latest/tutorial/process-model)提供 Chromium/Node 主进程及 renderer；这些平台特性支持上述候选收益，不证明本项目已经获得收益。[官方性能指南](https://www.electronjs.org/docs/latest/tutorial/performance)也强调先测量实际瓶颈。当前 Host/Provider 已有进程隔离，本地前端已有独立资源更新，因此不能将这两项能力完整计为新增价值。

对常驻后台产品，包体、进程内存、空闲 CPU 与更新维护成本需要持续承担。当前保留 Rust 的前提下，还会新增 Electron→Rust 的服务生命周期和调用边界；只有明确目标收益足够大时，这个长期成本才值得承担。

重启容器选型的条件：存在反复阻塞交付且已定位至 WebView 的问题；关键功能实质依赖 Electron/Node 能力；或桌面产品方向与团队技术结构明显转向复杂 React 工作台。先定义目标缺陷、可接受的资源预算和完成标准，再做前述 3–5 人日原型；原型收益不足则保留 Tauri。若需求仅是统一 UI 或修整大组件，优先沿用主题 token、组件边界和局部重构，避免把现有实现问题都归因于 Svelte。

## 推荐实现路径与模块影响

如果目标是使用 React 组件生态，先选 Tauri + React。如果目标包含统一 Chromium 渲染、需要 Node 能力或已确认的 WebView 限制，先做 Electron 原型。不能从既有 WKWebView 闪烁直接推断 Electron 一定解决问题。

完整迁移建议的数据流：React renderer → preload 的受控 Desktop API → Electron main。窗口/托盘/文件选择由桌面适配器处理；业务请求经本地 IPC → Rust 应用服务 → 现有 Host/Gateway → 独立 Provider。业务服务拥有数据、Provider 和连接生命周期，UI 重载不重启服务。优先评估 Electron main 使用 [Node child_process.spawn](https://nodejs.org/api/child_process.html#child_processspawncommand-args-options) 启动 Rust 可执行文件，通信使用有界 stdio 或本地 socket，并沿用已有业务协议、错误和事件身份。

| 模块 | 原因与处置 |
| --- | --- |
| Host、Provider、协议、SDK、task-lineage crate | 保留领域与协议；验证启动目录、环境变量、数据库路径与进程退出行为 |
| `src-tauri/src/lib.rs`、`runtime_gateway/`、`app/` | 抽出不依赖 AppHandle/State/Emitter 的装配、服务和事件端口；不是逐条把命令业务重写成 JS |
| `frontend/`、Vite、Svelte store、图标 | 重写组件和生命周期；复用纯 TS、CSS token、素材和业务类型；Mantine 若采用，先限于主窗口并映射 token，验证透明桌宠不受全局样式影响 |
| `pet/surface/`、`platform/`、Objective-C 抽屉 | 保留几何规则及可独立工作的系统算法；更换窗口对象、事件监听和提交适配器 |
| `app/webcontent*`、assets、sound | 移植资源协议、文件 URL、双窗口版本切换和回滚；保留 backendApiVersion 与完整性检查语义 |
| `scripts/`、Tauri 配置、CI、updater | 重建应用资源布局、sidecar staging、签名、公证与安装；确认旧 Tauri 用户如何升级到新安装包 |

Rust sidecar 适合业务隔离，但不能通过 IPC 传递一个指针，让另一个进程直接操作 Electron 的 NSWindow。依赖窗口身份和 AppKit 主线程的代码必须放在窗口所属进程，或改造成明确的跨进程服务；同进程窗口的 AX 特殊路径也要重验。若需要原生 addon，只为窗口扩展设置小边界，不把全部业务塞入 addon。

另一条可行路线是 [napi-rs](https://napi.rs/docs/introduction/getting-started) 把 Rust 暴露给 Node。Node-API 提供指定 API 级别的 ABI 稳定性，但各 OS/架构的构建、打包、线程和崩溃隔离仍需处理；不能笼统声称所有 Rust addon 每次升级 Electron 都要重编译。当前业务已有独立 Provider 进程，sidecar 更贴合既有拓扑。

## 核心风险及验证路径

1. **窗口体验不等价**：Electron 官方已有 macOS [panel 类型](https://www.electronjs.org/docs/latest/api/structures/base-window-options)及[鼠标透传](https://www.electronjs.org/docs/latest/tutorial/custom-window-interactions)，不需要预设所有能力都要自写 addon；仍须实机验证非激活、按需焦点、Space/全屏、像素命中、拖动、刘海安全区、多屏和 DPI。
2. **合成与闪烁**：`macos_frame.rs` 直接用 WKWebView/WKSnapshotConfiguration，无法直接移植至 Chromium；先验证是否仍需覆盖窗口和截图交接。沿用原生逐帧采集与像素分析方法，覆盖连续反向展开、收起、拖动；浏览器截图不能代替原生合成验收。
3. **进程生命周期**：新本地传输须验证取消、超时、背压、事件顺序、重放、重复订阅和后台服务退出。杀掉 renderer、sidecar、Provider，确认故障隔离、清理与重新订阅，不擅自重发发送/审批类写操作。
4. **权限与数据身份**：Rust helper 拆出后，不能假设已有完全磁盘访问、辅助功能授权自然适用。用正式签名安装包验证权限目标、设置、宠物库、数据库、配对身份和端口占用；保持数据路径，不无意创建第二套用户数据。
5. **更新断层**：现有 Tauri updater feed/签名与新容器不能假设互通。验证旧版本到新安装包的一次完整升级与失败恢复；两个窗口热更新后后台任务应保持。Electron 内置 [autoUpdater](https://www.electronjs.org/docs/latest/api/auto-updater) 不提供 Linux 支持，如新增 Linux 分发必须另选方案并验收。
6. **常驻资源成本**：保留 Rust 不会消除 Chromium/Node 和新增进程的开销。两套实际安装包在同机、同素材和同连接条件下测包体、总进程内存、启动到可交互时间、空闲 CPU、动画负载及待机功耗；未测前不给固定增加多少 MB 或百分比的结论。
7. **桌面调用边界**：以 [contextIsolation/contextBridge](https://www.electronjs.org/docs/latest/tutorial/context-isolation) 暴露具体业务动作，保留现有 main/pet 窗口权限检查；验证 Markdown 链接、路径和资源加载行为，避免为兼容旧调用开放任意 IPC。

## 验证计划与推进门槛

先用 3–5 人日做风险原型：Rust 独立启动后完成一次 Gateway 请求和事件订阅；Electron 同时打开主窗口与透明桌宠；验证 panel、透传、全屏 Space、拖动及高频展开收起；尝试正式签名的 helper 启动与权限路径；测常驻资源。原型用于决定是否继续，不能作为已可发布的交付。

通过后先收敛 Desktop API 和 Rust 服务边界，再按页面迁 React，并迁移主/宠双入口；继续保留 Surface 几何单写者和 revision 抗旧回复规则。可用旧 Svelte 资源验证 Electron 容器，再切 React，降低同时排查两套变化的成本。最终验收 macOS/Windows 安装、旧数据升级、权限、更新、远程链路与原生窗口。

测试复用入口：`frontend/lib/api.test.ts`、`frontend/lib/pet/surface.test.ts`、Surface 几何测试、Host/Provider 合同测试、`scripts/pet_*_ui_qa.mjs` 和原生 motion QA。Tauri mock 与 Svelte 挂载测试需要适配，协议与纯模型用例尽量保留。

## 本轮验证、未知项与知识演进

已运行 `git status --short`（评估前干净）、`git rev-parse --short HEAD`、`rg` 源码/文档检索及 Python 文件/行数/依赖引用统计；阅读了 API、Surface 附近测试。文档完成后执行 `git diff --check`。未安装依赖、构建 Electron 或运行应用测试；当前工作区缺少 `node_modules/.bin/vitest`。

尚未知：迁移动机及验收优先级、工程师实际熟练度、Electron 目标版本、各平台签名环境、Chromium 下是否仍有原生闪烁、sidecar 权限继承及实际性能。上述未知项使工期只能是规划区间。若只是为 Mantine 换容器，当前证据支持先迁 React；如决定迁 Electron，应以原型结果更新本文，再形成正式架构决策和平台验收记录。
