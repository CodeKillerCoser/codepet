# 原生顶部展开闪烁的像素排查

## 现象和证据

2026-11-10，用户反馈 `.5` 在 Mac 仍闪烁，与交付的浏览器动图不同。本机 LG ULTRAFINE，1920×1080 逻辑尺寸、2x、无物理刘海。隔离 `pet_surface_qa --capture` 使用生产 Surface 和顶部组件、自主循环展开收起；CUA 连续读取真实窗口截图，避免一次 click 返回时动画已结束。

`artifacts/native-flicker-before/cycle.json` 记录 171 张原生帧时间。65 帧接近空白，68 帧头像被裁，113 帧缩窗前背景中心偏移约 140 个物理像素。`pet_native_motion_images_qa.py` 对这一失败基线报出 26、65、68、107、113 帧异常。此前的浏览器截图与原生 DOM 坐标检查通过，不能反证这些实际像素。

## 根因与引入历史

`.5` 用 DOM 快照和 viewport 补偿协调布局，但 AppKit 改 frame、WKWebView 提交新像素并非同一时钟。resize promise 和 rAF 不是原生画面交接的确认。顶栏 clip 又与共享元素平移动画叠加；只去掉 clip 的中间版本仍出现头像先移动到黑色背景外，说明 compositor transform 与背景的 layout 宽高动画不同步。引入提交未确认。

## 修复及模块

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
