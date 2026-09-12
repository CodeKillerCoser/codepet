# 布局与尺寸

## 当前尺寸模型

桌宠窗口在 `frontend/PetApp.svelte` 中使用预设逻辑宽高。用户手动缩放被禁用，但应用仍会调用 `setSize()`，确保窗口 frame 可预测。

当前逻辑宽度为 320px（原为 360px），任务区最高 368px。CSS 宽度随容器收缩，卡片使用组容器的完整宽度；宠物传入的显示 scale 为用户设置值的 90%，仍受头像组件最小尺寸保护。保持预设窗口 frame，不以折叠内容高度反复调整原生窗口。

`PetActivityGroups.svelte` 按等待交互、进行中、已完成分组。收起时最前方直接显示真实任务气泡，最多露出两张后层气泡；使用 `perspective: 600px`，每层垂直错位 7px、深度递减 18px，并预留底部空间。点击顶层气泡或组按钮只展开本组，组按钮可再次收起。各组展开状态独立。

分组仅改变排列，不改变气泡的标题、消息预览、来源、状态和时间。运行中气泡继续使用个性化背景（含渐变）、边框色、边框跑马灯、背景呼吸和动画时长；收起和展开使用同一份卡片与配置。变换施加在外层 slot，避免干扰卡片动画。

任务卡片内容不应依赖固定卡片高度。回复编辑器和 footer 间距需要在卡片中自然撑开。

## 已知 UI 约束

- 只有一个任务卡片时，不应出现无意义列表滚动条。
- 操作按钮存在时，footer 间距仍需可见。
- 回复模式可以覆盖到桌宠窗口区域上方，但控件必须保持可触达。
- 文本不能覆盖卡片 footer 或按钮。
- Markdown 完整解析后，在桌宠气泡内裁剪为一行预览；展开本组也不出现消息详情或内部滚动区。长代码、表格不得扩大卡片宽度。
- 组按钮和最前层气泡参与鼠标透传命中；收起的后层使用 `inert`，不能获取焦点或参与命中。关闭按钮独立处理点击，不触发分组展开。

## 验证

- 人工检查单卡、多卡、回复模式、审批模式和收起任务列表。
- 自动化：`npx vitest run frontend/PetApp.test.ts frontend/lib/activityGroups.test.ts frontend/lib/markdown.test.ts frontend/lib/gradientColor.test.ts frontend/lib/petHitTest.test.ts`，31 项通过。
- 浏览器：配置 `CODEPET_QA_NODE_MODULES` 后运行 `node scripts/pet_groups_ui_qa.mjs`，验证独立展开、一行预览、颜色和动画、关闭动作、键盘焦点及 280/320/360px 宽度，通过。截图位于 `artifacts/task-lineage-qa/pet-groups/`。
- 原生桌面上的透明区域透传和屏幕边界仍需实机验收；浏览器检查不能代替窗口系统验证。

## 当前轮回复预览（2026-09-13）

气泡第二行只展示本轮最新助手回复，未收到正文时显示“正在思考中”。不以用户 prompt、错误信息、工具名、工具参数或 cwd 占位；第一行标题和第三行 harness、状态、时间保留原有行为。

Pet Gateway 的 projection 只将 last_assistant_message 写入 summary；用户提交新轮时清空旧摘要，并忽略提交事件可能携带的旧回复。工具/错误事件不覆盖已有助手正文，旧轮事件沿用 turn/time 边界拒绝。前端 Markdown 完整解析后限制一行，无内部滚动。

验证：Projection 的 6 项测试通过，包含首轮思考、工具内容不占位、同轮回复更新、新轮清空和旧轮迟到回复拒绝。浏览器检查覆盖单行高度、分组展开高度不变与原有颜色/动画/关闭/焦点行为。

本次交付：前端定向测试 19 项通过，浏览器含“正在思考中”占位断言复跑通过；webcontent 已安装到 data/webcontent/v4。完整 release + NSIS 构建成功，安装程序为 `src-tauri/target/release/bundle/nsis/Code Pet_0.3.9-beta_x64-setup.exe`。此修正包含 Rust 投影逻辑，需要更新原生程序，不能仅依赖替换 UI；未运行安装器。
