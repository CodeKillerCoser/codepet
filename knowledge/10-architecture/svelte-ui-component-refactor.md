# Svelte UI 组件接入与文件拆分

状态：2026-11-24，用户授权按 shadcn-svelte 推进，保留 Tauri/Svelte/Rust、现有主题、颜色和图标依赖。实现与验证记录如下；浏览器原生 IPC 使用隔离 fixture，尚未执行真实 Tauri 窗口验收。

## 背景与目标

现有 UI 已有 Radix Colors 和项目 token，但通用控件、弹窗焦点和 Tabs 键盘逻辑自行维护。`App.svelte` 为 2350 行，`styles.css` 为 3038 行。用户要求引入成熟组件、分析并迁移适合的自定义实现、保持视觉一致，并以 1500 行作为拆分审查点、2000 行为上限。

不迁移 Electron/React，不重做视觉、不替换图标，不更改 Provider/Gateway 协议和 Rust 业务。不在 UI 改动中混拆远程连接与 Provider 的 Rust 长文件。

## 现状证据与采用方案

`package.json`、锁文件及主题导入确认 `@radix-ui/colors` 3.0.0、`@lucide/svelte` 1.16.0 持续使用。本轮保持这两项依赖声明、锁定版本和完整性字段不变，也未修改主题色板。

`frontend/lib/ui/` 按 shadcn-svelte 的源码分发模式接入 Button、Input、Textarea、Switch、Tabs、Dialog，保留 MIT 许可和上游来源提交 `3c4152d7d3904e76f6f756ec8ffca7c9ec72f977`。保留组件的属性、ref、绑定、Snippet 与 Bits UI 交互，调整默认样式以沿用项目 CSS。运行时采用 Bits UI 2.22.0，Tailwind 4.1.18 只扫描 UI 目录，不加载 Preflight 或替换全局主题。工具类的语义颜色映射到已有 `--app-*` 和焦点 token。

Dialog 默认就地渲染以继承主窗口的主题上下文；Title/Description 通过 primitive 的 child 接口保留原来的 h3/p 元素，使既有排版选择器继续生效。独立 Portal 使用时必须显式考虑主题继承。组件更新方法和本地差异写在 UI 目录 README 中。

## 自定义组件与样式迁移清单

| 原有实现 | 本轮处置及原因 |
| --- | --- |
| App、导航、工具栏、设置页和弹窗中的普通按钮 | 迁移共享 Button，继续使用原 class、Lucide 图标、disabled 和业务回调；窗口拖动区域仍由原工具栏管理 |
| 通知设置的文本、URL、密码、数字、时间输入及文本域；信令地址 | 迁移 Input/Textarea，保留原生属性与保存时机，维持 label、校验和主题样式 |
| 主题气泡、通知和自启动开关 | 迁移 Bits UI Switch，保留 checked 保存行为，按原 34×20 轨道和 16px 圆点绘制；不替换颜色 token |
| `HorizontalTabs` 手写方向键/Home/End | 迁移共享 Tabs，保留路由、外部 tabpanel ID 与原横线样式 |
| 配对原生 dialog、手动焦点循环 | 迁移共享 Dialog；保留取消指定 pairing ID、复制/倒计时和业务控制的焦点恢复；删除旧 `dialogFocus.ts` 及其实现测试，以实际键盘回归覆盖 |
| 更新提示的手写 role=dialog/scrim | 迁移 UpdateDialog，增加受控焦点与 Escape；安装期间禁用关闭，保留 DMG/自动安装、失败重试和忽略版本行为 |
| 任务抽取历史 dialog | 拆出 ExtractionHistory，采用共享 Dialog，保留右侧抽屉布局与公开 `openHistory()` 入口 |
| SettingsRow、业务列表、任务关系图、消息、权限与 Provider 展示组件 | 保留业务组合及局部样式，内部原生控件不因新增库而机械重写；通用控件后续复用 UI 入口 |
| 原生 select、details、range/color 输入、Tauri 文件选择/确认框 | 保留已有浏览器或系统交互；普通库包装并不能减少其业务逻辑，原生颜色/范围编辑继续沿用已有测试 |
| 桌宠 Surface、DockPanel 动画、像素命中、动作轨道和窗口抽屉 | 保留原实现，通用 HTML 组件不能替代原生窗口事务与精确指针行为 |

## 模块拆分与影响

- `App.svelte` 从 2350 行降到 1278 行，保留路由、壳层、运行时与配对装配；外观和通知页分别拆到 `lib/settings/`，设置归一化进入 `model.ts`。移除已无 UI 引用的旧关键字编辑和展示 helper，保留兼容旧配置的归一化规则。
- AppearanceSettings 管理颜色编辑 revision、延迟保存和 pending。首次访问后保持挂载，切页不丢失延迟写入；显式 onChange/onSave 与 App 同步设置，旧保存回复不能覆盖新编辑。
- NotificationSettings 保留机器人配置、模板、渠道及测试通知。跨页面保存计数仍由 App 管理；busy 通知操作继续阻止资源重载。
- `styles.css` 改为顺序入口，基础/主界面/设置/桌宠/响应式分别进入五个文件，最大模块 1090 行，保持既有 CSS 顺序和有效规则。主窗口额外加载 UI 适配，桌宠入口不加载该样式；前端所有源文件均低于 1500 行。
- 历史抽屉原有的遮罩/阴影颜色从局部 raw literal 收到主窗口主题别名，数值保持不变；保留该抽屉无背景模糊、15px 标题的表现。移除已被共享 Overlay 替代的旧 modal-scrim/原生 backdrop 规则。
- `tsconfig.frontend.json` 提供生产前端的类型检查，测试与 QA fixture 分别使用它们自己的运行器。本轮补 Node/Vite 类型，并修正 ProviderKind 推断和 WindowDock 回调中可空目标的类型收窄，不改变业务行为。
- `check_source_size.mjs`、CI 和 AGENTS 记录文件长度规约，前端及新增/修改源码受硬上限约束。

## 全仓长文件 Review

前端已消除两处超长文件。全仓额外发现以下 16 个手写源/测试文件超过 1500 行，未在本轮做 Rust/协议生成器实现变更：

| 区域与文件 | 行数 | 建议拆分职责及验证 |
| --- | ---: | --- |
| `codepet-host/src/gateway.rs` | 2248 | 目录查询/交互写入/通知分发；Gateway 合同与超时测试 |
| `codepet-host/src/providers/manager.rs` | 2797 | 插件目录、实例生命周期、运行时选择；启动/退出/热加载回归 |
| `codepet-host/src/remote/access.rs` | 2496 | 配对、凭据、客户端状态；远程鉴权和撤销测试 |
| `codepet-host/tests/manager_gateway.rs` | 2180 | 按能力拆集成场景，fixture 共用；保留原用例全集 |
| `codepet-host/tests/remote_lan_listener.rs` | 1780 | 连接、认证、重连场景；网络竞态与端口回归 |
| `provider-claude/src/provider.rs` | 3460 | 目录、会话、turn/审批；Claude vertical tests |
| `provider-claude/tests/provider_vertical.rs` | 1636 | 按读取/写入/恢复拆场景；保持生命周期覆盖 |
| `provider-codex/src/client.rs` | 3215 | 握手、请求分发、事件与进程清理；App Server 传输测试 |
| `provider-codex/src/mapper.rs` | 1880 | 会话、消息、工具与审批映射；稳定身份与内容合同 |
| `provider-codex/src/provider.rs` | 4248 | 原生目录、会话与 turn 生命周期；分页/控制/恢复测试 |
| `provider-codex/tests/provider_vertical.rs` | 5236 | 按目录、消息和写入合同拆测试；完整运行原有场景 |
| `provider-opencode/src/provider.rs` | 3383 | 目录、SSE 与控制请求；OpenCode vertical tests |
| `src-tauri/src/agent/codex_desktop_ipc/client.rs` | 2486 | IPC 握手、订阅和投影；历史边界测试与当前使用情况复核 |
| `src-tauri/src/runtime_gateway/remote_access.rs` | 2754 | Tauri adapter 与配对业务 helper；配对/超时和公开 re-export |
| `src-tauri/src/runtime_gateway/tauri_bridge.rs` | 1530 | 运行时装配、事件桥和命令；本地事件/生命周期合同 |
| `tools/protocol-codegen/generate.mjs` | 2446 | schema、Rust/TS/Dart 输出与兼容检查；protocol:check |

前三组区域分别位于 `crates/codepet-host/`、`crates/providers/codepet-provider-*/`；表中缩写只用于阅读。生成的 SDK 类型、vendor 和独立示例 bundle 不按手写文件拆分。未来触及这些长文件时按 [源码长度规约](../60-rules/source-file-size.md)处理，不能将它们描述为已经满足上限。

## 风险与验证路径

视觉级联或字体变化：使用同一个生产 App fixture 对比改动前后亮/暗及 980×700、820×600 截图，并检查页面无横向溢出。焦点丢失：真实浏览器验证 Dialog 的 Tab/Shift+Tab/Escape、Tabs End 导航及配对按钮焦点恢复。保存回归：验证开关、渠道名称、模板、渐变角度和切页后延迟写入。桌宠回归：执行布局、动作可见性、透明背景和逐帧动画测试。旧配置回归：模型测试覆盖独立默认值、渠道保留、旧过滤器和外观边界。

## 验证记录与剩余项

基线 `npx vitest run frontend` 为 208 项通过。最终结果：

- `npx vitest run frontend`：39 个文件、208 项通过（删除 4 项旧手写焦点测试，新增 4 项设置兼容测试；焦点交给浏览器行为回归验证）。
- `npm run check:frontend`：生产前端 0 类型错误，保留 5 条原有 CSS 兼容/未使用选择器警告。此配置不将测试/QA fixture 纳入生产检查，不表示根 tsconfig 下所有历史夹具已清理类型错误。
- `npm run check:source-size`：前端及本次修改源码 0 超限；`--review` 列出上述 16 项历史审查项。
- `npm run build:webcontent`：主/宠双入口及清单构建通过，backendApiVersion 保持 21。接入后的主入口 JS gzip 约 91.96 kB，拆分后接库前约 61.43 kB；新增交互依赖有包体成本，未测启动时延或常驻内存。
- `npm run test:ui`：页面主题/尺寸、设置持久化、切页延迟保存、Tabs 键盘、配对复制/取消/焦点、抽取历史和更新弹窗全部通过。
- `npm run test:pet:ui`、`npm run test:pet:motion`、`npm run test:pet:stack`：布局、响应尺寸、动作可见性、焦点、展开/反向及卡片堆叠通过；`uv run --with pillow python scripts/pet_motion_images_qa.py` 的 312 帧像素分析通过。
- `CODEPET_QA_NODE_MODULES=<repo>/node_modules node scripts/app_updates_ui_qa.mjs`：DMG、错误重试、忽略/手动检查、自动安装、无更新及网络失败通过。
- `env TMPDIR=/private/tmp npm run test:webcontent`：11 项通过。默认 macOS 临时目录存在 `/var` 与 `/private/var` 别名，原测试路径字符串断言失败；改用规范临时目录运行通过，未改无关资源安装实现。
- `git diff --check` 及新增文件空白检查通过；核对两个受保护依赖的完整 lock entry 和声明均与 HEAD 相同。

各命令的 browser fixture 不访问真实应用数据。未执行 Rust 构建、签名/安装或真实 Tauri 多窗口验收，UI IPC 方法保持不变。

视觉对照中，通用、外观、通知、权限、亮暗主题及小窗口场景，在 RGB 容差 8 下没有差异；关于页约 0.06%（构建信息），更新弹窗约 0.77%（自动焦点轮廓等），已人工查看截图。原生窗口跨 Space、实际键盘焦点和 WebKit 合成仍应在发布前实机验收。

## 知识沉淀

组件库评估更新为已采用源码接入；本文件保存迁移/保留清单，UI README 记录来源和更新约束，源码长度规约及 AGENTS 约束后续开发。长文件的后端拆分是独立后续工作，不扩大本轮 UI 范围。

## 推送主分支前整合（2026-11-24）

用户要求提交并推送 main 时，远端已推进至 `29c6efb1`，包含 Agent 工作台/CodeMirror 与原生窗口拖出改动。重构提交重放到该基线，保留工作台入口、导航焦点、主题别名、依赖及全部原生窗口更新，不恢复主分支已移出的旧任务页入口。App 整合后为 1271 行。抽取历史组件改用独立 QA 场景验证，主窗口 QA 同时确认工作台入口仍可用。

整合后重新执行 `npm ci`、生产类型检查、源码长度检查、完整前端测试、双入口构建、资源测试、共享 UI 回归及 `scripts/codex_workbench_ui_qa.mjs`：40 个测试文件、213 项通过，生产类型检查 0 错误/5 条既有 CSS 警告；工作台搜索、筛选、CodeMirror 预览、主题、窄屏、焦点及应用壳层检查通过。资源测试仍使用规范 TMPDIR，11 项通过。

正式构建沿用新 main 的 backendApiVersion 23，不将其降回先前基线的 21。主入口 JS gzip 约 161.57 kB，包含新主分支的工作台/CodeMirror，不能将相对旧基线的整体差额归因于 UI 组件库。Radix Colors/Lucide 的声明与完整锁条目相对新 main 均未变更。
