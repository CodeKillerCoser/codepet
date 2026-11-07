# 布局与尺寸

## 当前尺寸模型

桌面模式窗口宽度为 320px，高度跟随实际内容；顶部模式收起按内容自适应，宽度至多 280px、高 44px，展开宽至多 640px、高度随任务列表自适应。用户手动缩放被禁用，前端测量交给原生 Surface 协调器调整 frame。不要按最大角色和完整任务区预留固定窗口高度；2026-11-08 的 584px 预设曾使单任务上方保留大块透明区域。

`frontend/lib/pet/surface.ts` 使用 ResizeObserver 观察 `.pet-content` 自然高度、角色中心及尺寸；内容保留顶部 10px、底部 12px 余量。布局测量去抖 60ms，并带原生 revision，旧请求不能覆盖新模式。普通悬浮保持角色中心的屏幕位置，内容超出工作区才整体钳制；顶部模式固定停靠区下缘与面板中心，独立的 44px 状态条 hover 后展开深色列表。拖动时任务暂时收起，原生保留鼠标抓取偏移；恢复任务时角色位置不跳动。旧右下角工具 `windowFrame.ts` 已移除。

任务区最高 368px，低高度屏幕额外保留 190px 角色/操作栏预算，其余内容在任务区滚动。操作栏的实际按钮数决定角色区最小高度，防止新增停靠按钮越出原生 frame。浏览器实测无任务 126px、单任务 243px，属于内容结果而非窗口预设。

桌面 UI 的 `PetActivityGroups.svelte` 按等待交互、进行中、已完成分组。收起时最前方直接显示真实任务气泡，最多露出两张后层气泡；使用 `perspective: 600px`，每层垂直错位 7px、深度递减 18px，并预留底部空间。点击顶层气泡或组按钮只展开本组，组按钮可再次收起。各组展开状态独立。

分组仅改变排列，不改变气泡的标题、消息预览、来源、状态和时间。运行中气泡继续使用个性化背景（含渐变）、边框色、边框跑马灯、背景呼吸和动画时长；收起和展开使用同一份卡片与配置。变换施加在外层 slot，避免干扰卡片动画。

任务卡片内容不应依赖固定卡片高度。回复编辑器和 footer 间距需要在卡片中自然撑开。

顶部 UI 独立使用 `NotchPet.svelte`：收起使用紧凑状态条，展开后顶栏与列表共享同宽容器；仅列表内部滚动，顶栏与动作保持可见。左右各 16px 装饰边距容纳顶部外侧圆角，不预留整屏透明区域。面板本身参与鼠标命中，透明区域仍透传；见[两套 UI 与共享状态](two-views-and-shared-state.md)。

## 桌面 UI 约束

- 正常无活动时不渲染任务区和“暂无任务”气泡，只保留桌宠；真实连接失败仍显示错误提示。
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
- `CODEPET_QA_NODE_MODULES=/tmp/codepet-pet-qa/node_modules node scripts/pet_window_frame_ui_qa.mjs`：真实布局配合模拟原生几何，覆盖内容贴合、底部锚点、收起/展开、顶部边界、滚动上限、负坐标屏幕、125% DPI 和 resize 事件稳定性。
- 原生桌面上的透明区域透传和屏幕边界仍需实机验收；浏览器检查不能代替窗口系统验证。

## 当前轮回复预览（2026-09-13）

气泡第二行只展示本轮最新助手回复，未收到正文时显示“正在思考中”。不以用户 prompt、错误信息、工具名、工具参数或 cwd 占位；第一行标题和第三行 harness、状态、时间保留原有行为。

Pet Gateway 的 projection 只将 last_assistant_message 写入 summary；用户提交新轮时清空旧摘要，并忽略提交事件可能携带的旧回复。工具/错误事件不覆盖已有助手正文，旧轮事件沿用 turn/time 边界拒绝。前端 Markdown 完整解析后限制一行，无内部滚动。

验证：Projection 的 6 项测试通过，包含首轮思考、工具内容不占位、同轮回复更新、新轮清空和旧轮迟到回复拒绝。浏览器检查覆盖单行高度、分组展开高度不变与原有颜色/动画/关闭/焦点行为。

本次交付：前端定向测试 19 项通过，浏览器含“正在思考中”占位断言复跑通过；webcontent 已安装到 data/webcontent/v4。完整 release + NSIS 构建成功，安装程序为 `src-tauri/target/release/bundle/nsis/Code Pet_0.3.9-beta_x64-setup.exe`。此修正包含 Rust 投影逻辑，需要更新原生程序，不能仅依赖替换 UI；未运行安装器。
