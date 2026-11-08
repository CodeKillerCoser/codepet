# 桌宠角色与动画模块

## 背景

用户希望替换原有桌宠外观，并授权清理旧实现。原 `PetSprite.svelte` 使用 24×30 固定矩阵；图片导入强制缩小到 16–128px；`PetAvatar.svelte` 混合素材读取和定时器，透明命中代码散落在窗口中。新实现保留 Svelte 和现有任务窗口，使用动画图集呈现角色。

## 目标

- 默认提供橘猫、熊猫，支持待机、思考、工作、等待、完成、失败及鼠标互动。
- 统一素材读取、动画状态、透明命中与失败回退，能独立测试。
- 支持动画包导入和清晰图片导入；保留自定义宠物、气泡、审批、透明度、抠图和声音行为。

## 非目标

不引入 React/Phaser/Live2D，不让宠物自主移动原生窗口，不改变 Provider/任务协议、气泡内容与分组。不自动安装或替换用户正在运行的 App。

## 现状理解

桌宠由 Pet Gateway 的 `pet.snapshot` 提供任务状态。窗口使用内容自适应高度、全局鼠标坐标轮询和 `setIgnoreCursorEvents`；CSS 的 `pointer-events` 不能替代原生透传。设置和桌宠共享同代外置 webcontent，原生命令变更需要升级契约。

素材来自 [CoPet](https://github.com/ChanceYu/CoPet/tree/e75658a22705c2f698289a0ff67e9fb652770e94)，使用该提交中的橘猫、熊猫 WebP 原文件。仓库 MIT 授权保存在素材目录；发行时的完整署名与许可位于 `public/third-party/copet/NOTICE.txt`，随资源清单分发。Svelte 组件为本仓库实现。

## 实现路径

角色采用 1536×1872 图集，8 列、9 行，每格 192×208；各行动只播放有效帧，不播放行尾空格。动画由每帧时长驱动，休眠后按真实经过时间定位。页面隐藏、减少动画偏好和静态缩略图均停止帧定时器；组件卸载清理定时器、监听器及未完成的素材请求。

业务状态和瞬时手势分开：等待输入/审批优先于失败、运行、完成、待机。抚摸播放跳跃，悬停挥手，拖动按方向播放行走；互动结束恢复最新任务状态，审批/失败不会被普通抚摸遮蔽。鞭子效果复用原声音和 SVG，角色短暂显示受惊动作。

素材切换带请求序号，晚到结果不能覆盖新角色；文件缺失、解码失败或图集尺寸错误回退默认橘猫。外部图片通过原生命令读取为 data URL，使 WebView 可以读取 alpha。单文件最多 16 MiB。命中检测使用当前行列和自然图片尺寸，静态图片的 contain 留白不命中；情绪符号不扩大命中范围。

内置 ID 为 `default` 和 `builtin:panda`，不能删除。运行时只保留 builtin、image、codex-atlas 三种角色类型；像素矩阵组件、配色类型/默认值、像素化参数/控件、旧图片迁移分支和专用主题变量均已删除。设置使用 Serde 默认的未知字段忽略行为，未知角色类型统一回退 builtin，下一次保存只写当前模型。库规范化仅恢复内置档案和有效选择，不复制或删除用户素材。已导入图片继续使用记录的路径；需要换成高清原图时重新导入。

动画包选择 `pet.json`，只接受包目录内的 PNG/WebP 图集，拒绝越界路径、逃逸软链接、错误尺寸和超限文件。导入生成唯一目录与 ID，失败清理本次目录，删除只清理受管理的导入目录。自动发现仍读取配置宠物目录和 `~/.codex/pets`；不经网络加载素材。

图片导入保留 alpha、颜色和原始分辨率，超过 1024px 时等比平滑缩小；抠图是显式的单独入口。目录设置继续遵循 Path Manager：切换目录不自动搬迁或删除旧数据。

## 涉及模块

- `frontend/lib/pet/`：负责角色资源、动画状态、Svelte 渲染、命中和宠物库 UI；替代旧头像、矩阵和 atlas 文件。
- `frontend/PetApp.svelte`：只组合任务状态、窗口 API、角色与原有气泡/动作栏；高度按角色和任务区实际布局计算，具体约束见 [布局与尺寸](layout-and-sizing.md)。
- `frontend/App.svelte`、`frontend/lib/api.ts`：连接宠物库操作、保存大小和新导入命令；移除像素化控件。
- `src-tauri/src/pet/{library,images,packages}.rs`：分别负责配置/选择、图片处理、动画包验证与复制。
- `src-tauri/src/app/settings.rs`：当前角色数据模型、内置档案和未知类型回退；`src-tauri/src/lib.rs` 注册命令并通知另一窗口。
- `webcontent-contract.json`：API 3 升至 4，防止旧原生程序加载依赖新命令的 UI。必须同时更新二进制与资源包。

## 风险与验证

- 动画越格、后台耗电、异步串图：`animation.test.ts` 和 `PetAvatar.test.ts` 覆盖有效帧、计时、减少动画、卸载、晚到请求与失败回退。
- 透明区域或图片留白拦截：`hitTest.test.ts` 覆盖 atlas 偏移和 contain；浏览器以真实素材验证透明/不透明坐标、透传命令和拖动调用。
- 配置丢失、包损坏或覆盖原件：Rust 集成测试覆盖未知字段读取/保存清理、默认保护、非法包、软链接、重复导入、源目录删除后重新发现、图片 alpha 和尺寸。
- 大小、焦点、按钮和气泡回归：浏览器检查 280/320/360px 桌宠及 2/3/5 倍大小，动作栏不能覆盖角色；独立复跑原气泡分组检查。

## 测试计划与结果

2026-11-07 已执行：

- `npx vitest run frontend`：215 项通过；后续局部变更的 70 项相关测试复跑通过。
- `npm run build`：通过，图集由 Vite 打包；最终 API 4 清单已包含两份图集和第三方许可。
- `TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo test --manifest-path src-tauri/Cargo.toml --test pet_library_tests --test settings_tests --test subject_cutout_tests --test macos_window_tests`。仓库已知 feature/config 检查冲突需此环境覆盖；不修改生产配置。macOS 测试 include 路径修正为实际 `platform/macos_window.rs`；共 34 项通过（宠物库 17、设置 12、抠图 2、窗口 3）。
- `CODEPET_QA_NODE_MODULES=/tmp/codepet-pet-qa/node_modules node scripts/pet_module_ui_qa.mjs`：通过，截图在 `artifacts/pet-module-qa/`；该脚本默认 Chrome，可通过 `CODEPET_QA_BROWSER` 改浏览器。
- 原 `scripts/pet_groups_ui_qa.mjs` 在同一 Playwright 路径、`CODEPET_QA_BROWSER=chrome` 下通过。
- `TMPDIR=/private/tmp npm run test:webcontent`：11 项通过。默认 macOS 临时路径含 `/var`→`/private/var` 别名时，原排序测试存在路径字符串断言差异。
- `npx tsc --noEmit` 仍被仓库既有 Node 类型、活动代码和旧测试 fixture 诊断阻塞；新增宠物文件未产生诊断。

### 2026-11-08 清理与预览构建

用户进一步要求彻底删除旧方案并生成可运行版本。本轮删除了生产代码中的 `PixelPetSprite`、`Palette`、`defaultPetSprite`、`imagePixelSize`、旧像素图迁移和 `--asset-pet-sprite-*`。测试与浏览器 fixture 使用当前模型；仅设置回归测试保留废弃字段作为输入，验证它们不会破坏其他设置且不会重新序列化。历史图片文件不做删除或重写。

验证结果：`npx vitest run frontend` 215 项、上述 Rust 四个集成测试 35 项、宠物浏览器 QA、`TMPDIR=/private/tmp npm run test:webcontent` 11 项及 `npm run version:check` 均通过。未知类型采用 Serde 的通用回退，不保留旧角色类型或专用迁移实现。

完整构建需要 Bun；本机 PATH 缺失，使用 `/tmp/codepet-pet-build-tools` 安装与 CI 相同的 Bun 1.4.2。构建命令为 `PATH="/tmp/codepet-pet-build-tools/node_modules/.bin:$PATH" npm run tauri -- build --bundles app,dmg --config src-tauri/tauri.macos-dmg.conf.json --config '{"version":"0.3.9-beta+pet.20261108","bundle":{"createUpdaterArtifacts":false}}'`。构建版本只通过临时配置传入，不修改仓库基础版本；使用本地 ad-hoc 签名，不运行 CI 证书导入脚本。

构建成功，Apple Silicon App 与 DMG 位于 `/Users/wangxin/Downloads/codepet-pet-preview-20261108/`。`CFBundleShortVersionString` 为 `0.3.9-beta+pet.20261108`，`codesign --verify --deep --strict`、`hdiutil verify` 和包内全部 16 个 webcontent 文件 SHA-256 检查通过，确认包含两份角色图集与许可。DMG SHA-256 为 `05f82dbe857f1a892d98e29a3ecf3e32bedf5e3643d1b297a82de949bbc129bc`。未安装、启动或公证；原生交互仍待实机验收。Tauri CLI 自动加入的全局 `macos-private-api` feature 已恢复，源码继续使用原有 macOS 专属依赖配置。

## 知识沉淀

同步宠物个性化、窗口尺寸和体验文档；透明命中规约新增“命中与渲染共享当前帧、静态图片需扣除留白”的要求。

## 未知项

浏览器模拟 IPC 不能证明操作系统实际透传、原生拖动及跨屏 DPI 行为。尚未安装新 App，也未完成 macOS/Windows 实机跨屏和安装包验收。新版动画包目前仅支持上述 8×9 契约，不支持任意 atlas 布局或 Live2D。
