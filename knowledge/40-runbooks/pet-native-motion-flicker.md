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

### 从 GitHub 安装后只见激活按钮或尺寸提示

GitHub Actions 的 macOS DMG `0.3.11-beta+34dba6d` 内包含窗口收纳原生模块和接口 19 的桌宠页面。排查先核对运行中进程的可执行文件路径，再检查该包的 `CFBundleVersion` 和资源目录中 webcontent manifest 的 `backendApiVersion`。本次检查时本机实际运行的是同一 bundle identifier 的接口 17 测试包。用户遇到钉钉问题时的进程版本和原始报错尚待确认。

接口 20 起，在日志中过滤 `pet.window_drawer`。从一次 `drop matched window` 向后检查展示阶段。

- `resize-panel` 表示 AX 回读的窗口比请求的区域大。先按窗口实际尺寸扩容再对齐。屏幕容纳不下时，提示同时给出所需尺寸和可用尺寸。`fit failed` 记录请求与回读的位置、大小，可用于区分位置被钳制与尺寸超限。
- `awaiting-surface` 表示窗口列表还无法确认原窗口在目标位置。桌宠封住展示区并短暂加快复查。若原窗口编号已消失，只在同一进程内同时确认唯一几何匹配及原 AX 窗口身份之后更新编号。
- `covered` 表示窗口已找到且全部被遮挡。显式展开时会核对激活后的层级，后台复查不会抢焦点。`visible` 表示已找到可展示的区域。

本地回归运行 `macos_drawer_parking.m` 的 `--notifications` 分支及 `npm run test:pet:ui`。在已解锁的目标 Mac 上运行同一原生程序的 `--fit` 分支检查 AX 写入与回读，再在实际钉钉窗口核对尺寸、拖入后直接显示及释放后的位置。锁屏下的模拟窗口栈、AppKit 通知和浏览器回归无法验收第三方窗口的实际合成画面。

### 拖入后全黑，以及 Code Pet 主窗口漏接松手

2026-11-19 的桌面日志中，e90cc8e 加载了三次。三次窗口匹配后共记录五次 `covered requested 800x560 measured 800x560`。旧包没有记录遮挡者，无法从这份报告辨认具体浮层。用户另外确认拖动的是 Code Pet 主窗口，未看到松手提示。

按窗口栈从前往后查找。找到桌宠窗口编号后，清除此前累计的遮挡，再检查桌宠与目标之间的窗口。目标先于桌宠出现，或列表缺少桌宠时，保留之前的保守检查。真实遮挡发生时，状态变化日志中的 `coverOwner`、`layer`、`window` 是第一个参与遮挡的窗口，`host` 和 `target` 可用于核对顺序。多个遮挡者可能共同盖满展示区。

同一进程的主窗口由 AppKit 本地鼠标监视器接收事件。旧实现只在全局监视器里向前端发布松手序号。现由原生手势统一记录，AppKit 开始移动普通窗口时补记来源，两次前端采样之间已经结束的手势也能提交一次。通过同进程 AX 操作主窗口，第三方窗口仍按系统辅助功能授权处理。显式展示时，如果 Code Pet 持有前台焦点，macOS 14 及以上可将激活权交给目标应用。

本轮运行原生窗口测试和 `npm run test:pet:ui`。原生用例覆盖桌宠前、桌宠后以及缺少桌宠的遮挡者、重复松手和还原。在隔离 Tauri 进程内加载实际主窗口页面，回放按下、AppKit 窗口移动及松手，生产桌宠页面最终打开了 **820 × 600 点**的原生交互区。此项验证覆盖快拖回放，持续按住标题栏时的提示仍需物理鼠标验收。

## 第三个窗口经过应用页出现黑块，拖出后又被收回

2026-11-24 用两个 AppKit 进程复现夹层。独立目标窗口在后，另一个普通窗口在中，桌宠样本在前。旧的裁剪计算会从目标足迹减去中间窗口的矩形，这一块显示为桌宠的黑底。焦点或空间通知还会在收到新窗口栈之前清空整个开孔。用户办公电脑上每块遮挡的实际来源尚未采样。

应用页发现夹层或宿主落到目标后方时，调整宿主的原生层级及相对顺序，再读一次 WindowServer。窗口栈已经正确时只读。焦点变化先撤销鼠标透传，等新结果更新裁剪；停靠和释放仍须先封闭画面。复查时将另一个窗口放在展示区的一角，观察它能否正常覆盖整个组合，并检查收起后顶栏是否回到原有层级。

旧同步路径仅让出边角缩放，纯位置变化可能触发回写展示区坐标。现在按下的窗口编号、光标位移、窗口位移和尺寸共同识别拖出；正常拖出移除收纳条目，保留松手位置。快速松手时，收起及投放入口也会先处理该手势。主动点击 Dock 的释放按钮仍恢复收纳前几何。

在登录图形会话的 Mac 上运行原生样本，再运行 `npm run test:pet:ui`。原生样本 `checkGroupedWindows` 比较修复前后三个真实窗口的层级与可见面积，`checkDragOut` 覆盖拖动中解除、快速松手、收起竞争与同次手势重复投放。UI 样本分别检查待确认期间的裁剪和输入、拖出后 Dock 条目消失、原生缩放及显式释放。
