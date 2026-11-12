# 桌面对话 UI 选型建议

状态：2026-11-11 用户确认电脑端始终使用原生 App，本期暂不接入桌面对话，亦不引入 shadcn-svelte。以下必要性与技术评估作为备选资料保留，不代表当前实施计划。

## 产品必要性与手机端复用

2026-11-11 用户指出分页、实时合并、切换隔离等已经在手机实现，要求先判断功能是否值得接入。此前把桌面缺少代码列为待设计问题不够准确：需要迁移既有行为，不能重新发明同步和交互规则。

### 本轮证据

只读核对 Remote 提交 [`eb64de788172e7748474615e0b787d627d9a3194`](https://github.com/CodeKillerCoser/codepet-remote/tree/eb64de788172e7748474615e0b787d627d9a3194)：

- `lib/application/conversations/conversation_detail_controller.dart` 已包含 reload、快照事件窗口衔接、loadEarlier、interaction、interrupt、resolveApproval、send 和队列控制。旧请求通过 epoch/lease 等身份防止跨会话或连接失效后回写。
- `lib/application/conversations/conversation_message_cache.dart` 的消息源在页面离开后继续接事件；按路由缓存，带前插分页状态、scope/version、LRU 和闲置清理，Provider 变化可定向失效。
- `lib/application/sessions/device_session.dart` 管理连接、消息缓存、交互权暂停/恢复、输入草稿和队列投影等会话级状态。
- 本仓库 `frontend/lib/TaskLineage.svelte` 已能打开线程并继续输入，但展示消息来自 `lineageApi.messages` 的原始消息查询；`LineageMessages.svelte` 是正文与证据位置列表，不是完整 Gateway 实时会话视图。`ConversationComposer.svelte` 已有可复用输入逻辑。
- `README.md` 和 `knowledge/20-product/remote-control-and-plugins.md` 将远程控制与 Provider 扩展设为主线；桌面对话应服务同一 Host 的会话连续使用。

源码检查证明存在相关实现，不代表本轮重新运行了手机测试或完成真实端到端验收。

### 判断

建议接入范围明确的桌面对话能力，理由是：

1. 手机上经 Host 发起或继续的工作，回到电脑后可以在同一 Gateway 中查看与操作，避免仍用手机输入长指令和阅读长输出。
2. 统一 Provider 的收益可以延伸到桌面，不要求用户为不同 runtime 分别寻找操作入口；尤其是不具备独立图形界面的 runtime。
3. 已有任务谱系支持“回溯原文、继续线程”，完整实时会话可补上发送后观察执行、审批、修正和验收的闭环。

这些是从现有能力与产品目标推导的价值判断，目前没有桌面对话使用频次、原工具切换成本或用户留存的量化证据，不能宣称是最高优先级或已验证的增长需求。如果使用者始终在原生工具处理电脑端会话，且不用 CodePet 的任务跟进和统一 Provider 入口，功能增益会降低。

桌面对话与采用 shadcn-svelte 是两项决策。前者有明确场景；后者仅解决展示与交互组件，不因手机已有业务逻辑而自动成为必要依赖。建议以已有任务回溯入口和统一会话入口为载体补齐查看、继续、流式、审批、中断、队列操作，首先验证实际闭环，再决定展示组件的收益是否覆盖 Tailwind、主题映射和运行时依赖成本。

### 复用边界与验证路径

- 手机的 controller/cache/DeviceSession 及相应用例是行为参照；Flutter/Dart 不能直接被当前 Svelte 页面 import。若继续使用 Svelte，应迁移必要的纯业务逻辑与测试场景，复用生成协议和 Host 权威状态；不为此先重构一套跨语言共享运行时。
- 桌面本地 Tauri 调用适配已有 Gateway，无需复制手机的发现、配对、TLS/WSS 等网络准入；业务状态行为保持一致。若未来要管理其他电脑，另行评估桌面远程连接范围。
- 新页面与已有任务入口应复用同一个桌面会话状态源，避免重复缓存和发送控制。用两入口切换、后台输出和旧异步返回测试验证。
- 与同一 Host 的手机会话连续使用，不等于与原生 Codex Desktop/CLI 保证无缝共享控制权。沿用当前 Provider capability 和交互权规则，验证写冲突、主动释放与重新获取，不绕过锁。
- 优先验证“手机发起 → 桌面查看/继续 → 手机收到更新”和“任务节点 → 原会话 → 执行反馈/审批 → 返回任务”两条真实路径，覆盖结果未知、跨端已审批与能力缺失。根据实际使用反馈决定是否扩展成完整桌面工作台。

本轮仅更新评估文档，运行 `git status --short`、源码静态阅读、GitHub 只读查询及 `git diff --check`；未实现应用功能或运行测试。

## 背景

手机已有远程对话，桌面需要接入完整对话界面。此次只评估可嵌入的开源 UI。

仓库证据：`package.json` 使用 Svelte 5、Vite 6、Tauri 2，未声明 React、Tailwind 或 shadcn-svelte。`frontend/lib/codepetGateway.ts` 已通过 `codepet_gateway_request` 调用生成的 Gateway 协议；附近测试覆盖响应 ID 校验与协议错误。`ConversationComposer.svelte` 已有能力判断、草稿、输入队列、steer 和发送结果待核对逻辑，并在 `LineageMessages.svelte` 中被引用。因此可以复用业务基础，但不能将现有原始消息面板视为完整远程对话客户端。

手机消息行为依据 [Remote 消息数据源规约](../60-rules/remote-conversation-message-sources.md)，本次未读取独立手机仓库源码。

## 决策

建议优先验证 **shadcn-svelte AI Elements**，按需引入展示组件，继续由项目 Gateway 和消息 store 管理协议状态。这是候选方向，不是已批准的依赖迁移。

## 备选方案

| 候选 | 证据与收益 | 适配成本 |
| --- | --- | --- |
| [shadcn-svelte AI Elements](https://shadcn-svelte.com/docs/ai-elements) | Svelte 组件组合，可复制源码；官方说明可接受任意消息来源，包含 Chat、Tool、Confirmation、Queue 等模式 | [Vite 接入](https://shadcn-svelte.com/docs/installation/vite)需要 Tailwind；映射项目主题和 Gateway 状态 |
| [Svelte AI Elements](https://github.com/SikandarJODD/ai-elements) | Vercel AI Elements 的社区移植，提供消息、流式 Markdown、推理与工具 UI；MIT | 同样依赖 shadcn-svelte，需逐组件确认依赖和维护情况；不与前一候选混称 |
| [assistant-ui](https://github.com/assistant-ui/assistant-ui) | MIT；[ExternalStoreRuntime](https://www.assistant-ui.com/docs/runtimes/custom/external-store)可连接已有状态和自定义消息格式 | 此路线使用 React，增加双框架挂载、主题和生命周期维护成本 |
| [Deep Chat](https://github.com/OvidijusParsiunas/deep-chat) | MIT；框架无关组件，[自定义 handler](https://deepchat.dev/docs/connect/)支持自有请求和流式回调 | 适合快速聊天入口；项目的工具卡、审批、Diff、队列和历史前插需单独验证定制能力 |

shadcn-svelte 为 [MIT](https://github.com/huntabyte/shadcn-svelte/blob/main/LICENSE.md)。引入源码时保留许可声明，具体传递依赖在选定版本后检查。

## 取舍理由

现有 Svelte 技术栈和自有 Gateway 是主要约束。UI 应消费消息投影并发出业务动作，不能为适配演示而将协议改成普通聊天 HTTP 接口。审批外观不等于审批业务能力，队列外观也不能替代现有入队幂等和待核对行为。

## 影响范围

- `frontend/lib/codepetGateway.ts` 与 Gateway SDK：复用请求边界，增加会话 store 所需的调用与事件适配；不手改生成物。
- `ConversationComposer.svelte`：保留发送、队列与草稿语义，评估拆出控制逻辑以接新外观。
- 新对话页面和消息 store：负责历史分页、实时增量、canonical 替换、路由隔离和能力驱动操作。
- `frontend/lib/theme/`、构建配置：映射主题 token，控制新样式对已有页面的影响。

## 后续观察

先验证一个闭环：会话列表、读取历史、流式回复、停止、工具调用、审批。风险与验证路径：

- 消息丢失或重复：用历史分页与 delta 并发、canonical 原位替换、终态不重拉、断线恢复场景验证 store。
- 队列或审批错发：复用 Composer 测试风格，覆盖能力缺失、切会话、重复点击、结果未知和审批已解决。
- 样式与交互回归：检查明暗主题、窄窗、键盘焦点、中文输入法、按钮可见性、上翻阅读时流式输出及历史前插锚点。
- WebView 兼容性与性能：在实际 Tauri 窗口验证长会话、长代码块、Diff 和持续流式输出，再确定组件版本与包体积。

本次执行 `git status --short`、源码与附近测试静态阅读、官方文档及仓库查询；只新增本文，未运行应用测试或安装依赖。尚未验证真实手机协议与桌面新 store 的端到端一致性，也没有给出实现工期承诺。原型结果应回写本文后再形成正式选型。

## shadcn-svelte 深入评估

2026-11-10 按用户要求进一步评估第一个方案。结论：适合用作桌面对话的展示基础，接入复杂度中等；完整的会话同步、审批和恢复仍是项目自身的工作。建议进入有限范围验证，尚不能声称生产可用。

### 源码证据与依赖

通过 GitHub connector 读取的上游提交为 [`3c4152d7d3904e76f6f756ec8ffca7c9ec72f977`](https://github.com/huntabyte/shadcn-svelte/tree/3c4152d7d3904e76f6f756ec8ffca7c9ec72f977)，以下为该提交源码，不代表 npm 发布包已逐文件验证一致。

- [MessageScroller Provider](https://github.com/huntabyte/shadcn-svelte/blob/3c4152d7d3904e76f6f756ec8ffca7c9ec72f977/docs/src/lib/registry/ui/message-scroller/message-scroller-provider.svelte)只是 `@shadcn-svelte/primitives/message-scroller` 的封装。因此“复制组件源码”不等于没有运行时依赖。
- [primitives package](https://github.com/huntabyte/shadcn-svelte/blob/3c4152d7d3904e76f6f756ec8ffca7c9ec72f977/packages/primitives/package.json)版本为 `0.0.1`，Svelte peer 为 `^5.33.0`，直接依赖包括 `@tanstack/virtual-core`、`runed`、`svelte-toolbelt`、`esm-env`。本仓库 `package-lock.json` 锁定 Svelte `5.55.9`，满足此 peer；这不替代实际安装与编译检查。
- 检查的 vega registry 中，Button 使用 `tailwind-variants@^3.3.0`，Trace 声明 `bits-ui@^2.20.0` 等依赖，代码高亮 hook 使用 `shiki@^3.20.0`。实际闭包随选取组件和 style 变化，不能把整个文档站的依赖都当成接入必需项。
- [Markdown 封装](https://github.com/huntabyte/shadcn-svelte/blob/3c4152d7d3904e76f6f756ec8ffca7c9ec72f977/docs/src/lib/components/markdown/markdown.svelte)引入 `svelte-streamdown`，代码块另接 CodeBlock 与高亮 hook。Message 本身不自动提供完整 Markdown 渲染；照搬 Chat 示例还需要整理文档站内部 import。

### 行为适配

| 需求 | 可复用部分 | 项目仍需实现 |
| --- | --- | --- |
| 文本、推理、工具过程 | Message、Bubble、Trace、CodeBlock | 按 `ConversationItem.kind` 直接投影；保留 turn/item/content 稳定身份，不强制转换整套 AI SDK 状态 |
| 实时输出 | 渲染和滚动展示 | 将 `turn.outputDelta` 按 itemId/contentId 合并，canonical item 原位替换，处理乱序、去重和失效 |
| 历史前插 | Viewport 的 `preserveScrollOnPrepend` 默认开启 | 按现有规约手动分页，稳定 messageId，验证分页与流式输出并发 |
| 审批 | Item + Button 的 Confirmation 模式 | 绑定 Gateway 审批身份、合法 decisions、pending/resolved/expired 状态，处理手机已审批后的按钮更新 |
| 发送、队列、steer、停止 | InputGroup、Button 等外观 | 复用 Composer 的幂等请求、草稿、待核对结果及 capability 判断；停止绑定 Gateway interrupt |
| 长会话 | 官方提供 virtualizer 组合示例 | 显式接入虚拟化并测动态行高；默认消息列表不能视为已虚拟化 |
| Diff、文件资源、未知 item | 可以在消息中组合自定义内容 | 编写文件改动视图、资源打开动作、截断提示和 unknown 安全回退 |

Gateway 请求走 `codepetGateway()`，实时事件复用 `codepet-gateway-event`。本地窗口可通过已有 Tauri bridge 访问 Gateway，不需要为组件另加聊天服务。AI SDK 的 Chat、演示 transport、自动审批续跑和 regenerate 不是接入必需项。

### 滚动、样式与 Tauri 风险

[类型声明](https://github.com/huntabyte/shadcn-svelte/blob/3c4152d7d3904e76f6f756ec8ffca7c9ec72f977/packages/primitives/src/lib/message-scroller/types.ts)中 `autoScroll` 默认 false，需要显式开启跟随；默认打开位置为 end，前插保位默认 true。切会话应重建该会话的滚动上下文，重新打开时恢复项目保存的位置。

[上游浏览器测试](https://github.com/huntabyte/shadcn-svelte/blob/3c4152d7d3904e76f6f756ec8ffca7c9ec72f977/packages/primitives/src/lib/message-scroller/message-scroller.browser.test.ts)覆盖前插保位、末条增长、用户意图取消跟随、折叠内容及卸载重挂。这增加了选型信心，但本次只阅读测试，未执行。滚动实现使用 ResizeObserver、IntersectionObserver 和 requestAnimationFrame，未见这一模块要求 Node 服务；实际 WebKit/WebView2 行为仍需 Tauri 验证。

样式建议仅在主窗口聊天入口加载新增 Tailwind utilities，先关闭其全局 Preflight，使用可配置的前缀或隔离策略，并相应调整复制源码。将 background、foreground、muted、border、primary 等语义变量映射到项目 theme token；弹层若 portal 到 body，也必须得到同一 token。原因是当前 `styles.css` 同时承担应用全局基础样式，Vite 又有 main/pet 双入口，全局 reset 可能改变设置页和透明宠物窗口。验证要覆盖 980×700 默认窗、820×600 最小窗、亮暗主题、宠物透明背景与输入焦点。

代码高亮与 Markdown 需要独立验证：按需打包语言与主题，测长代码块持续增量时的耗时；确认 HTML、链接及资源打开策略继续满足项目现有 DOMPurify/Tauri 边界。此处没有认定上游存在安全缺陷，具体渲染配置尚未审计。

### 推荐验证顺序与通过条件

1. 锁定来源提交与运行时包版本，搭建 Message、MessageScroller、Trace 的静态 fixture；完成主题和双窗口样式检查，记录构建产物变化。
2. 接现有 Gateway：会话列表 → 最近一页 → 手动加载旧页 → delta/canonical 合并 → 终态保留输出；跑状态模型测试，覆盖路由切换和断线恢复。
3. 复用输入逻辑，接审批与停止；验证手机和桌面同时操作时不重复发送、不对已解决审批再次操作，中文输入法不误提交。
4. 在真实 Tauri 主窗口用长会话、长 Diff 和持续输出验证滚动锚点、虚拟化、键盘操作及内存，再确认正式采用。

按工程判断，组件和外观接入成本低到中，消息数据源与恢复成本中到高；后者也是采用其他 UI 库仍需完成的工作。本次没有实测包体积、吞吐、峰值内存或工期，不能给出可靠节省比例。

本轮检查：官方文档、上游固定提交源码与测试、本地协议类型及 Composer 附近测试的静态对照；`git diff --check`。未安装依赖、修改应用源码或执行应用测试；当前工作区没有 `node_modules/.bin/vitest`。终端 Git clone/raw 获取遇到连接问题，源码已通过 GitHub connector 读取。
