# 桌宠顶部透明留白阻止窗口上移

## 现象

2026-11-08，用户试用角色重构预览包后反馈：只有一张运行中任务气泡，窗口上方仍有很大一片透明区域；桌宠拖到屏幕上方时提前停止，像有不可见内容顶住了边界。

## 证据

- 用户截图中，任务气泡和熊猫靠下显示，红框标出了从屏幕上沿到气泡上方的空白。
- 修复前 `frontend/PetApp.svelte` 使用 `22 + ceil(32 × 5 × 208 / 192) + 12 + 8 + 368 = 584` 的固定高度；无任务或单任务也调用同一尺寸。
- `.pet-window` 高度为 `100vh` 且 `justify-content: flex-end`，实际内容靠下。`clampWindowPositionToMonitor()` 以整个原生窗口计算工作区边界，因此透明顶部到达上沿后，窗口无法继续上移。
- 原测试甚至明确断言固定最大尺寸；原浏览器 QA 的 `inner_size` 没有真实返回值，且不模拟 `set_size` 后的视口变化，因此只能检查内容布局，不能证明原生窗口与内容贴合。

## 根因与引入历史

固定最大 frame 与底部布局组合形成了无内容占用的原生窗口区域。鼠标透传只能解决点击穿过，不能消除窗口边界。旧版本已有固定 frame；本次角色重构又按最大角色尺寸扩大预留高度。涉及改动尚未提交，无法提供独立修复前 commit。

## 修复方案及涉及模块

- `frontend/PetApp.svelte`、`frontend/styles.css`：增加自然高度的 `.pet-content`，通过 ResizeObserver 监听实际布局，去抖后设置原生高度；任务区通常最高 368px，小屏根据工作区剩余空间进一步缩小。
- `frontend/lib/pet/windowFrame.ts`：使用物理像素保持右下角锚点，再做工作区约束；避免任务出现、折叠后桌宠无故跳动。原生拖动期间推迟尺寸调整。
- `frontend/qa/pet-module.ts`：增加可变原生尺寸、位置、显示器和缩放模拟，set_size 实际驱动 Playwright 视口变化。
- `scripts/pet_window_frame_ui_qa.mjs`：验证无任务、单任务、展开收起、顶部限位、小工作区滚动、负坐标显示器、125% DPI、键盘焦点和尺寸稳定性，替代仅断言源码包含固定预设的测试。

## 验证结果

`npx vitest run frontend`：215 项通过。`CODEPET_QA_NODE_MODULES=/tmp/codepet-pet-qa/node_modules node scripts/pet_window_frame_ui_qa.mjs` 与同环境 `scripts/pet_module_ui_qa.mjs` 均通过。默认 scale 的实际浏览器布局为无任务 126px、单任务 235px；后者截图位于 `artifacts/pet-module-qa/window-single-task.png`。125% 缩放允许最多 1 个物理像素的坐标舍入差。多任务展开后列表内部滚动，resize/move 后不会持续触发尺寸变化。

修复预览版本 `0.3.9-beta+pet.20261108.2` 已完成 macOS Apple Silicon 的 App/DMG 构建，保存在 `/Users/wangxin/Downloads/codepet-pet-window-fix-20261108/`。App 深度签名、DMG 校验及包内 16 个资源 SHA-256 均通过；未安装或启动。DMG SHA-256：`9da7d855bd1d25796a5eebeb05cca8bcfca53bb303663ef7284f2446ffb221a7`。本轮只改前端布局与测试，后端 API 契约仍为 4。

补充机制检查：`scripts/pet_window_frame_ui_qa.mjs` 增加角色 scale 增大/恢复、关闭最后一张任务卡及重新出现任务的场景，均验证原生模拟窗口自动调整并保持底边，通过。高度是测量结果，126/235px 并非固定档位；宽度仍为 320px，不应将当前实现描述成宽高均自适应。进程路径检查发现用户当时仍运行下载目录 `codepet-pet-preview-20261108` 的初版（Info.plist 为 `0.3.9-beta+pet.20261108`），修正版位于独立目录 `codepet-pet-window-fix-20261108`，版本后缀为 `.2`。

2026-11-08 01:00（本地时间），按用户要求改用独立前端更新：`npm run build:webcontent` 后执行 `npm run install:webcontent -- --data-dir '/Users/wangxin/Library/Application Support/code-pet'`，安装至 `versions/webcontent/v1`，API 4、版本 `0.3.9-beta`、builtAt `1794070841176`，16 个文件哈希校验通过。在运行中的 App“关于”点击“加载最新”，界面显示构建时间 01:00:41 和“已重新加载前端资源”，URL 带同一 builtAt；日志确认 hot loaded v1。进程 PID 18322 和原生 App 路径未变，未重启、未替换二进制。此次窗口修正只需热加载前端即可预览，无须安装 `.2` 整包。

## 回归防线

风险必须对应验证：顶部空白用内容边界与实际视口比较；位置跳变用调整前后物理底边比较；跨屏错误用负坐标和不同 DPI；反馈循环用稳定后的 set_size 调用次数；任务内容裁剪用小工作区内部滚动及键盘操作。同步了布局、多屏、产品体验和透明透传规约，禁止用固定大 frame 为未出现的内容预留空间。

## 未知项

浏览器模拟原生 IPC 不能证明 macOS/Windows 窗口管理器、跨屏拖拽及真实点击透传行为；修复包仍需在用户桌面验收。
