# 前端资源热加载

## 背景与目标

旧的资源读取器在启动时创建只读快照，更新数据目录后必须重启整个 App。现在“设置 → 关于 → 前端资源版本”提供“加载最新”，允许保留 Rust 后端、Provider 和任务服务，只重新加载主窗口和桌宠页面。它是生产资源切换，不是 Vite 的模块热更新；页面本地状态会重新初始化。

## 实现路径

`webcontent_reload::ReloadableAssets` 实现 Tauri Assets，并与托管状态共享同一个 RwLock 保护的资源代。`webcontent_info` 返回当前原生资源代及能力；`load_latest_webcontent` 只接受主窗口调用，使用独立互斥锁阻止并行切换，耗时磁盘操作在 blocking worker 上执行。

加载候选继续复用 `webcontent::resolve_directory`：包内与当前 data/versions/webcontent/vN 共同参与，先检查契约、入口、SemVer、builtAt、全文件 SHA-256，再按版本和构建时间选择。损坏候选被跳过并记日志，无可用候选时返回错误；相同或更旧的资源不降级、不刷新。

确认完整候选和桌宠 URL 后，原子替换内存资源代，再让桌宠导航到带构建时间查询参数的原页面。导航请求失败则恢复旧资源代。主窗口收到命令响应后才导航，避免刷新提前销毁 IPC 返回通道。Tauri 2.11.2 的生产协议忽略查询参数解析资源路径，新的查询参数用于避免命中旧 HTML 缓存。两个窗口切换共享资源，但刷新不是同时完成的。

旧页面可能在窗口刷新间隙请求延迟加载资源，因此保留之前已校验的非入口资源，当前资源优先；index.html/pet.html 始终只取当前资源代。此缓存保留到进程退出，频繁加载多个资源包会增加内存占用。哈希校验与导航请求成功不等于新 JavaScript 已完成启动，本版尚无页面健康确认或运行时白屏自动回滚。

## 页面与状态

`AboutSettings.svelte` 显示当前页面构建时嵌入的版本和时间，避免用原生版本或已切换的后端资源代冒充页面版本。Vite 同时生成 `build-info.json`，清单生成器读取相同 builtAt；安装保留该时间。`frontend/lib/webcontent.ts` 封装命令、资源代比较及一次性的会话内重载提示，刷新后回到关于页。

抽取配置以保存成功的 JSON 为基线判断未保存状态，保存中也阻止加载；用户可以通过提示返回抽取设置保存或重新加载配置。设置异步保存和相关前台操作未结束时禁用加载按钮。加载过程中主界面 inert，避免等待期间又输入新内容。后端任务不停止；列表选中、滚动等页面状态不保证保留。

## 兼容边界

首次启用需要升级原生程序。两个新增命令是可选能力扩展，未改变现有命令及资源包格式，因此继续使用 backend API 2；旧 App 仍可加载新前端，其关于页捕获能力查询失败后禁用“加载最新”并提示更新原生程序。开发模式不执行生产资源切换，继续使用 Vite。

安装脚本仍然只创建新 vN 槽位，不覆盖旧目录或签名包。普通窗口刷新仍不会重新扫描磁盘；有新资源时应通过“加载最新”，也可退出并重新启动 App。原生程序本身的更新仍需要重新启动进程。

## 涉及模块与验证

- Rust：`app/webcontent_reload.rs`、`app/webcontent.rs` 和 `lib.rs` 负责共享资源、命令、启动装配。原生测试覆盖跨克隆资源切换、旧延迟资源保留、导航失败恢复及重试、同版和旧版不刷新；原有加载测试继续覆盖损坏、契约、路径和候选排序。
- 构建：`vite.config.ts`、`scripts/webcontent.mjs` 负责共享构建时间。生产构建后检查 `build-info.json` 与 manifest 的版本/时间完全一致，并重新校验所有文件哈希。
- 前端：关于页、抽取表单和 App 负责入口、草稿阻止、操作中屏蔽、刷新后回到关于页。前端测试验证同版本不同构建触发刷新和提示只消费一次；模拟 IPC 页面验证无更新、草稿阻止/保存后解除、校验失败和旧原生能力缺失。

已运行 `npm run build:webcontent`、`npm run test:webcontent`（10 项）、`npx vitest run frontend`（原有 196 项）和新增 webcontent 测试（2 项）。原生 `cargo test --manifest-path src-tauri/Cargo.toml --features custom-protocol app::webcontent --lib --offline -j 2` 共 14 项通过。原生构建设置 TAURI_CONFIG 为 tauri.bin.conf.json，复用主工作区 target 缓存；未修改源码发布配置。

浏览器 QA 的 `webcontentCase=error/unsupported/reload` 为模拟 IPC 场景，仅用于界面和导航验证，不替代真实 WebView2、macOS 多窗口与后台任务持续运行的安装验收。

Windows Release 构建通过：`cargo build --manifest-path src-tauri/Cargo.toml --features custom-protocol --bin code-pet --release --offline -j 2`，耗时 11m05s。复用 target 下的 `release/code-pet.exe` 是本轮原生程序产物；未替换正在运行的安装版。前端 13 个清单文件已安装到用户数据目录 `versions/webcontent/v4`，版本 0.3.9-beta，builtAt=1789299203925，安装后全包校验通过。
