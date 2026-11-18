# 原生顶部展开闪烁的像素排查

## 现象和证据

2026-11-10，用户反馈 `.5` 在 Mac 仍闪烁，与交付的浏览器动图不同。本机 LG ULTRAFINE，1920×1080 逻辑尺寸、2x、无物理刘海。隔离 `pet_surface_qa --capture` 使用生产 Surface 和顶部组件、自主循环展开收起；CUA 连续读取真实窗口截图，避免一次 click 返回时动画已结束。

`artifacts/native-flicker-before/cycle.json` 记录 171 张原生帧时间。65 帧接近空白，68 帧头像被裁，113 帧缩窗前背景中心偏移约 140 个物理像素。`pet_native_motion_images_qa.py` 对这一失败基线报出 26、65、68、107、113 帧异常。此前的浏览器截图与原生 DOM 坐标检查通过，不能反证这些实际像素。

## 根因与引入历史

`.5` 用 DOM 快照和 viewport 补偿协调布局，但 AppKit 改 frame、WKWebView 提交新像素并非同一时钟。resize promise 和 rAF 不是原生画面交接的确认。顶栏 clip 又与共享元素平移动画叠加；只去掉 clip 的中间版本仍出现头像先移动到黑色背景外，说明 compositor transform 与背景的 layout 宽高动画不同步。引入提交未确认。

## 修复及模块

以下记录原生覆盖窗口方案的引入过程。当前前端已改为 `DockPanel.svelte` 与私有 `dockPanelMotion.ts`，删除 `notchMotion.ts`、逐业务节点动画和 DOM 克隆；原生 begin/end 协议不变。当前边界、验证命令和内容更新回归见[组件决策](../10-architecture/dock-panel-motion-boundary.md)。

- `PetApp.svelte`：状态机输入立即更新，只有面板尺寸的显示等待动画交接；不得把整个旧 Presentation 延迟回写。完整 UI QA 曾检出 Escape 后背景 hover 重新展开，因排队动画覆盖了新 hovered/focused 状态，分离后需重跑该回归。
- `notchMotion.ts`、`NotchPet.svelte`：去掉顶栏动态 clip；共享元素保留原字号/图像大小，以 left/top 与背景一起绘制。新建动画先暂停，原生完成交接后播放；收起先完成动效再缩窗。取消和反向从当前可见状态接续；原生尺寸交接期间合并到最近一次请求，不能中途采样新旧坐标。
- `platform/pet_surface/macos_frame.rs`：WKSnapshot 的 afterScreenUpdates 获取旧画面，在独立、不可交互、无边框的原生覆盖窗口中保留；临时隐藏原窗口 alpha 后调整 frame，快照窗口不参与 resize。新布局快照完成后恢复实时内容、移除覆盖。token 防旧回调移除新覆盖，失败/超时恢复可见性。纯移动时覆盖随窗口移动。
- `platform/pet_surface.rs`、`surface/mod.rs`、`surface.ts`：统一 begin/end 接口，平台差异留在平台层；AppKit resize 使用 NSAnimationContext 事务；原窗口交接完成再恢复 alpha、关闭覆盖窗口。
- `webcontent-contract.json`、命令注册、QA mock：API 12，要求完整 app 更新，防止前端调用旧后端缺失命令。没有改 Provider 数据或角色导入。

## 复查步骤

1. `npm run build:webcontent`；以 `TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo build --manifest-path src-tauri/Cargo.toml --example pet_surface_qa --features custom-protocol` 编译隔离原生 fixture。
2. 用测试 bundle 启动 `--capture`。需要反向时加 `--capture-reverse`，循环包含正常、110ms 反向、1ms 准备期反向。通过 CUA 连续截图保存 `cycle-<i>.png` 和 `cycle.json`（每项 i/t 毫秒）。测试数据在临时目录，不改真实任务。
3. `uv run --with pillow python scripts/pet_native_motion_images_qa.py <截图目录>` 检查每帧背景中心、头像像素，生成 GIF、变化帧对比图和报告。检测器针对居中、2x、熊猫 fixture；不同角色/DPI/位置必须调整验证条件，不能直接套阈值。
4. 逐张看变化帧：有无整帧空白、头像裁切、背景与标题错拍、最终缩窗偏移。GIF 仅用于回看，原 PNG/时间戳保留。
5. 运行顶部 UI QA 覆盖焦点、拖动冻结、任务列表、导入角色、响应式；native `--motion` 补充 AppKit/DOM 坐标和反向回归。最后替换 Applications 并核对运行路径、真实任务 UI。

## 验证与剩余风险

中间版单窗口连续 161 帧未报错，但扩大到反向场景后仍报空白和偏移，因此不能宣称已验收。最终版使用独立覆盖窗口，CUA 的原窗口截图不包含覆盖层，342 张截图中的空白不能直接代表桌面最终画面；不得将该 GIF 当作最终桌面录屏。尝试经 QuickTime/系统截屏 UI 录制桌面，当前工具未能启动录制，完整桌面合成逐帧验收仍缺失。

最终版原生 `--motion` 的正常展开/收起/反向共 129 帧通过；16 项 Surface、19 项前端定向、顶部 UI QA、浏览器居中/左右边缘/反向逐帧坐标、11 项 webcontent 检查通过。上述检查证明几何和交互回归，不替代最终桌面合成验收。完整 App 交付信息记录在顶部吸附文档。

物理刘海、混合 DPI、其他平台仍需对应设备验证。CUA 原生坐标拖动能力有限，浏览器拖动与状态机测试不能冒充真实鼠标链验收。快照失败恢复、连续反向不能遗留不可见 WebView，后续回归应保留。回归规则已补到 `knowledge/60-rules/pet-window-click-through.md`。

## 2026-11-18 应用页收纳原窗口

本轮在隔离的 Tauri Debug 进程中使用生产桌宠页面、WKWebView 和一个专用 AppKit 窗口。旧版静止 6 秒仍执行 88～89 次窗口同步。每次都经过 WebView IPC、AX 边界读取及 WindowServer 遮挡检查。旧版展开先画 260 ms 面板动画，再等下一次定时器定位原窗口。收起先移走原窗口，剩下的动画显示空面板。两段串行解释了可复现的延迟。真实桌面出现空白的具体帧仍需合成录像判定。

| 隔离样本 | 旧版 | 第一轮事件驱动版 |
| --- | ---: | ---: |
| 静止时原生同步次数/秒 | 14.75 | 1.94 |
| 桌宠宿主 CPU | 5.82% | 4.99% |
| WKWebView 内容进程 CPU | 4.10% | 4.27% |
| 两进程合计 CPU | 9.92% | 9.26% |
| 展开至首次确认原窗口可见，中位数 | 425 ms | 63 ms |
| 收起至面板进入 closed，中位数 | 348 ms | 36 ms |
| 各次切换的最大 rAF 间隔，中位数 / 全部最大值 | 38 / 44 ms | 23 / 48 ms |

将同一窗口拖入后固定展开，静置 650 ms，再取三段各 6 秒样本。用宿主和对应 WK 内容进程的累计 CPU 秒增量除以墙钟秒，同一核心占满计 100%。六次切换用同一按钮触发，展开终点为展示区出现经校验的可见区域。外部应用、WindowServer 与 GPU 开销不在表内。长尾仍有一帧 48 ms，短样本不足以推断日常耗电。逐轮原始数据见[旧版](../../artifacts/window-drawer-performance/baseline.json)和[第一轮优化](../../artifacts/window-drawer-performance/event-driven.json)。

应用页现在用 AX 通知、宿主 AppKit 通知与 Space/应用切换通知唤醒同步。主 RunLoop 合并同一轮通知，真实拖动触发即时复核。静止时每 500 ms 校验一次，停靠后注销应用页监听。原生层复用已协商的窗口尺寸，读宿主位置时只取面板和主屏几何。前端只在可见矩形发生变化时更新遮罩与鼠标命中，前一次异步结果不得覆盖较新的遮挡。窗口定位和面板最终尺寸在同一次原生快照交接内完成。收起先提交遮罩再移动原窗口，给两次 WebView 绘制回调留出时间，最长等 80 ms。上述保护以及后加的原生尺寸缓存与轻量坐标读取晚于表中 A/B 采样。

终版另做了两组可在锁屏时运行的分段测量。真实 AppKit 样本窗口以约 60 Hz 水平移动 40 次，从设置位置到宿主原生回调中位 0.62 ms，P95 为 0.84 ms。独立 Chromium 运行生产 Svelte 组件并模拟 Tauri IPC，静止 18 秒得到 36 次校验，三条 renderer 进程 CPU 合计 2.28%。40 次模拟位移通知至组件发出同步命令，中位 0.80 ms，P95 为 1.10 ms。两组测量各自覆盖不同环节，外部窗口处理 AX 和 WindowServer 呈现时间需真机另测。数据见[AppKit 回调](../../artifacts/window-drawer-performance/final-host-move-ms.json)与[终版前端](../../artifacts/window-drawer-performance/final-web.json)。

复查时用单独的原生样本窗口，保留相同的窗口尺寸、Debug/Release 配置和通知负载。前后各采 CPU 和切换时序，录像另跑一轮，避免把录屏开销混入 CPU。用一扇有明显颜色的窗口拖过展示区，连续捕获桌宠与原窗口的合成画面，同时检查快速反向和点击遮挡处。锁屏或显示器休眠时，将这轮标为无效。本次终版桌面像素采样因锁屏未完成。浏览器遮挡竞态、原生事件和 Surface 几何测试可独立运行。

- `npm run test:pet:ui` 验证移动后剩余可见区域和异步同步过期；`npm run test:pet:motion` 覆盖通知页居中、边缘和反向动效。
- `clang -fobjc-arc -framework AppKit -framework ApplicationServices -framework ImageIO src-tauri/tests/macos_drawer_parking.m src-tauri/src/platform/window_drawer/watch.m -o /tmp/codepet-drawer-parking-test` 后执行 `/tmp/codepet-drawer-parking-test --notifications`，验证真实面板移动、Space 失效通知和收起后注销。解锁后执行同一程序不带参数，验证完整窗口与停靠链路。
