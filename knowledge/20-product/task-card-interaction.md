# 任务卡片交互

## 卡片内容

任务卡片展示标题、消息、provider/source 元数据、状态，以及可用时的终止时间。展示辅助逻辑位于 `frontend/lib/activity.ts`。消息由 `frontend/lib/MarkdownMessage.svelte` 和 `frontend/lib/markdown.ts` 渲染 Markdown，支持段落、标题、列表、引用、代码块、链接和表格；普通文本仍可直接显示。

## 分组展示

- 为减少桌面遮挡，任务按“等待交互、进行中、已完成”分组，分组辅助逻辑位于 `frontend/lib/activityGroups.ts`。
- `waiting-approval` 与 `failed` 归入等待交互；`thinking` 与 `running` 归入进行中；`done` 归入已完成。失败任务保留自身失败状态，不能显示为完成；没有对应事件的组不展示。
- 每组默认收起，以 Z 轴透视堆叠展示：前景显示组名、数量、首个任务标题及状态，最多露出两张后层卡。后层向下轻微错位，沿负 Z 轴递减，数量角标始终反映真实任务数。
- 点击分组或用键盘 Enter/Space 展开，显示该组完整任务卡；可独立收起。组内沿用活动列表顺序，实时消息更新不会重置仍存在分组的展开状态。
- 分组状态是当前窗口的原生 `details` 状态，不写入设置；窗口重载、整份列表卸载或组消失后再次出现时，恢复默认收起。
- 正文为可聚焦、可选择的独立滚动区域，最高 120px；长代码行和表格在区域内横向滚动。点击标题仍打开来源会话，点击消息正文用于阅读和选择，链接通过系统默认应用打开。
- Markdown 输出经过 DOMPurify 标签及属性白名单清理，只允许 HTTP、HTTPS 和 mailto 链接；图片展示替代文字，不加载远程图片。渲染不改写原始事件，也不会把普通 JSON 自动变成任务摘要。

## 活动归并

卡片按 provider 加 session id、cwd 或全局 fallback 分组。同一个 activity key 的 active 更新会替换旧卡片。终态事件只有在属于已有可见活动，或能通过 fallback 匹配时才保留。

## 操作

- 打开：尝试激活来源应用或项目路径。
- 移除：从桌宠列表中移除任意任务卡片。
- 移除已完成：清除已完成卡片。
- 回复：只有 provider capability 判断安全时才显示。
- 审批：只在等待审批事件上显示。

## 回复模式

回复模式是 `frontend/PetApp.svelte` 内的本地 UI 状态。它可以打开和关闭，会聚焦 textarea，并让编辑器按内容自适应到最多五行。

## 风险

- 给 footer 增加操作按钮可能压掉底部间距，或造成无意义滚动条。
- 固定任务卡片高度很脆弱，因为回复编辑器、footer 和消息内容都会变化。
- 折叠组头必须纳入窗口鼠标命中区域，隐藏卡片不得拦截鼠标；检查 Windows/macOS 原生窗口透传。
- 变窄时审批按钮与状态可能争夺宽度；footer 允许换行，检查 320px 与 260px 布局、Tab 可达性及回复焦点。
- 运行中任务不应显示回复入口，因为后端除已验证控制路径外，无法可靠向 active provider session 注入消息。

## 验证

- 运行 `npx vitest run frontend/lib/activity.test.ts`。
- 修改 `frontend/PetApp.svelte` 时，人工检查单卡、多卡、回复模式和审批模式布局。
- 2026-09-12 浏览器模拟 Tauri IPC 验证：折叠与透视层次、组展开、Markdown 列表/代码/表格、实时更新保持展开、260px 窄窗口、深色主题、键盘展开、回复框自动聚焦。模拟验证不代表原生跨窗口透传或真实审批/回复链路已验证。
- 自动化命令：`npx vitest run frontend/PetApp.test.ts frontend/styles.test.ts frontend/lib/activity.test.ts frontend/lib/activityGroups.test.ts frontend/lib/markdown.test.ts frontend/lib/petHitTest.test.ts`；覆盖分组迁移、Markdown 格式和危险内容清理。
- 2026-09-12 上述 111 项测试通过，`npm run build` 与 `git diff --check` 通过。
- `npx tsc --noEmit` 仍报告既有问题，包括 `activity.ts` 的可选值、旧测试 fixture 缺字段及 Node 类型缺失；不能宣称完整类型检查通过。

## 主工作区 Pet Gateway 适配（2026-09-12）

当前 `PetApp.svelte` 消费 `petSnapshot()` 的 `PetTask`，不再直接消费旧 `PetEvent` 或旧回复/审批接口。折叠组和 Markdown 复用相同展示组件；等待授权、等待输入、失败和状态未知进入等待交互组，运行进入进行中组，完成和中断进入结束组。空快照继续显示来源连接状态。当前卡片保留本地隐藏与清除已完成操作；上文旧回复模式及其浏览器验证是迁移前记录，不能视为当前 Gateway 动作能力。

合并后前端 196 项测试通过；新增 Gateway 状态分组覆盖，并检查组头加入原生窗口命中选择器。实际原生透传与 Gateway 联动仍需人工验收。
