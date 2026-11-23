# Svelte 5 组件库评估

状态：2026-11-24，用户先决定暂不迁移技术栈，随后授权按 shadcn-svelte 接入并重构 UI，保持主题、图标和视觉。已采用源码接入模式，见 [实现与验证](../10-architecture/svelte-ui-component-refactor.md)。以下保留选型时的调研证据，不再表示未获授权。沿用 living-dev-doc-writer 的证据、范围和验证写法。

## 背景与现状证据

当前为 Tauri + Svelte + TypeScript + Vite。`package-lock.json` 锁定 Svelte 5.55.9、Vite 6.4.2；`package.json` 未引入 Tailwind 或通用交互组件库。`@radix-ui/colors` 只提供颜色，不能视为已有 Dialog、Select、Menu 等交互组件。

用户追问后复核：此前引入的主题体系仍然存在并接入生产入口，不能把“未接入通用交互组件库”描述成“项目没有 UI/主题基础”。Git 历史 `79a38a67`（2026-06-03，`feat(theme): migrate UI styling to theme tokens`）涉及 Radix Colors 引入；当前 `styles.css` 导入 `theme/tokens.css`，主窗口和桌宠入口均加载 `styles.css`，并分别调用 `themeClassNames()`。`theme/main-window.css` 继续提供主窗口语义角色，`frontend/styles.test.ts` 有主题接入约束测试，本轮只读未执行。

现有层次为 Radix Colors 基础色板 → 项目主题 token/亮暗主题 → 自有 CSS 与 Svelte 组件；另有 `@lucide/svelte` 图标库。后续候选应补齐交互与组件复用，并沿用现有主题。`@radix-ui/colors` 不等于 Radix UI 交互组件或 Radix Themes 成品组件系统。查询当前可见 Git 历史的根 `package.json` 未发现 Bits UI、shadcn-svelte、Skeleton、Flowbite、Melt UI 或 SvelteUI 的依赖变更记录；这一范围不能证明其他仓库或不可见分支从未引入过组件库。

`frontend/lib/theme/README.md` 与 `knowledge/30-domains/theme-system/token-model.md` 定义项目 CSS token，存在 `--app-*` 和 `--pet-*` 两套别名。`HorizontalTabs.svelte` 自行处理方向键/Home/End，`dialogFocus.test.ts` 覆盖自有焦点循环。引入成熟库的可见价值是减少这类通用交互、键盘操作和焦点行为的维护，并提供统一控件外观；它不会实现 Gateway、权限或原生窗口业务。

历史 [桌面对话 UI 选型](desktop-chat-ui-selection-proposal.md) 记录过 shadcn-svelte，但当时用户未决定接入。本轮是通用 UI 选型，不自动开启桌面对话功能。

## 候选与成熟度证据

本轮阅读官方文档、更新日志/仓库发布页，并通过 Python urllib 直接请求 npm registry，核对 latest、发布时间、peerDependencies 和 license。下表仅为当日快照，发布时间来自 registry 的 UTC 字段；满足 peer 不等于已通过本仓库集成测试。

| 候选 | 已核验发布与兼容性 | 形态与成本 |
| --- | --- | --- |
| [Bits UI](https://www.bits-ui.com/docs/introduction) | `bits-ui` 2.22.0，11 月 21 日发布；peer Svelte `^5.33.0`，另需核对 `@internationalized/date` peer；MIT | 无预设视觉的交互基础组件，包含 Dialog、Select、Menu、Tabs、Tooltip 等。提供键盘与焦点机制，无 CSS reset，不强制 Tailwind；需自己用现有 token 编写外观 |
| [shadcn-svelte](https://www.shadcn-svelte.com/docs) | CLI `shadcn-svelte` 1.8.1，11 月 11 日发布；CLI peer Svelte `^5.0.0`；MIT；[更新日志](https://www.shadcn-svelte.com/docs/changelog)有近期功能更新 | 基于 Bits UI 与 Tailwind，把选定组件源码加入项目；可改外观，需维护本地源码与选定组件的运行时依赖。CLI 版本不代表所有组件的运行时要求 |
| [Skeleton](https://www.skeleton.dev/) | `@skeletonlabs/skeleton` 与 `skeleton-svelte` 5.0.6，11 月 19 日发布；Svelte peer `^5.40.0`，Tailwind `^4.0.0`；MIT | 设计系统加现成组件，主题、明暗色及布局配套完整。适合希望成套使用现成 UI 的方向；要协调其全局基础样式与项目 token |
| [Flowbite Svelte](https://flowbite-svelte.com/docs/pages/introduction) | 1.33.1，4 月 7 日发布；Svelte peer `^5.40.0`、Tailwind `^4.1.4`；MIT | 成品控件与后台页面常用元素较丰富，适合快速搭建配置/管理界面。距离最近发布较长，不能仅据此判定停止维护；选用时再核对目标组件问题与修复情况 |

Registry 核对入口：`https://registry.npmjs.org/bits-ui`、`https://registry.npmjs.org/shadcn-svelte`、`https://registry.npmjs.org/@skeletonlabs%2Fskeleton`、`https://registry.npmjs.org/@skeletonlabs%2Fskeleton-svelte`、`https://registry.npmjs.org/flowbite-svelte`。本轮四项主候选均声明兼容当前锁定的 Svelte 版本；实际构建及逐组件依赖尚未验证。

## 建议与备选方向

当前最适合深入验证的是 Bits UI 与 shadcn-svelte：

- 保留当前视觉、CSS 和主题体系，优先 Bits UI，逐步替换复杂弹层、选择器、菜单等交互。基础按钮和简单布局不必为了统一依赖全部重写。
- 需要现成统一外观并愿意引入 Tailwind，优先 shadcn-svelte，先在主窗口按需接组件。它与 Bits UI 属于上层组件和底层交互的关系，不需要把两套完整实现重复包装。
- 希望获得更完整的现成主题/设计系统，可进一步比较 Skeleton；需要后台风格现成页面元素，可比较 Flowbite。两者均需 Tailwind，不应同时套多套全局设计系统；[Skeleton 官方安装说明](https://www.skeleton.dev/docs/svelte/get-started/installation)明确说明其与 Flowbite/DaisyUI 的部分 Tailwind 定义冲突。

以上为项目适配判断，不是四个库的性能或质量排名。Mantine 官方明确[仅支持 React](https://help.mantine.dev/q/vue-svelte-angular)，无需为此改变当前技术栈。

## 涉及模块与验证路径

- `package.json`、锁文件、Vite：仅实际接入时修改，确认 Svelte peer 与按需依赖；shadcn、Skeleton、Flowbite 需要增加 Tailwind，当前为 Vite 前端，不需为组件库迁入 SvelteKit 服务端。
- `frontend/lib/theme/` 与全局 CSS：将库语义颜色映射到现有 token，检查 Tailwind Preflight、字体和尺寸是否改变旧页面。新增样式先限制在主窗口入口，验证透明桌宠的背景及布局。
- `frontend/lib/` 与 `App.svelte`：用一个真实 Dialog/Select/Menu 作为候选样本，检查打开焦点、Tab/Shift+Tab、方向键、Escape、关闭后焦点恢复；保留动作能力判断。
- Pet 与 Tauri 原生层：不接管 Surface、NSPanel 或窗口抽屉。库的 HTML Dialog/Drawer/Floating Panel 不等于操作系统原生窗口。
- Portal 弹层：确认挂载目标可继承主题、层级正确；透明桌宠的小窗口边界可能裁掉弹层，使用原生窗口尺寸验收。

选定后跑限定范围的 Vitest、`npm run build:webcontent`，并在真实 Tauri 主窗口验证 980×700 与 820×600、亮暗主题、键盘和中文输入法；桌宠检查透明背景、透传及展开状态。使用 Tailwind 4 时按目标 OS/WebView 范围核对其 CSS 能力要求。先完成代表性组件集成，再估算批量替换成本。

## 验证记录与未知项

本轮执行 git 状态检查、源码/知识检索、锁文件版本读取、registry 只读请求，以及官方文档查询。未安装依赖、改应用代码或运行 UI/构建测试；完成后检查文档空白与本地引用。仍需明确是保留现有外观，还是希望整套现成风格，以及目标组件清单和支持的 OS/WebView 版本。正式采用后在本文更新版本、实测结果和取舍，不把候选调研视为已经完成选型。
