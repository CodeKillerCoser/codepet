# Host 启动后残留 Provider 未就绪诊断

## 现象

Host 连接页的本机运行时卡片显示 `provider-unavailable`，详情为 `provider_plugin_unavailable: Provider plugin is not ready: dev.codepet.claude`。插件稍后成功启动，卡片仍保留旧错误。其他 Provider 也可能受相同启动时序影响。

## 证据

- 2026-09-05 安装版日志 `logs/code-pet.log`：23:59:19.218 的 `frontend.main.list_agent_runtimes` 在 3ms 内完成；23:59:20.961 才打印三个插件初始化成功的批量结果。Claude 插件进程仍在运行，后续 `provider.ping` 持续成功。
- 修复前 `ProviderHostState::runtime_views` 对尚未 Ready 的插件调用 Runtime API，manager 返回上述错误；`App.svelte::refresh` 只在启动时加载一次库存。
- 新增的 `ProviderConnectionStatus` 只更新连接标签，不更新卡片使用的 `agentRuntimes`；因此连接状态恢复不能清掉早期诊断。

## 根因

后台插件初始化和前端首次查询并行，前端把暂时未就绪的结果作为永久快照。连接状态与依赖它的 Runtime 列表缺少失效通知处理。这份错误本身不能证明 Claude CLI 未安装，也不能证明 Provider 进程已退出。

## 引入历史

首次引入的提交未确认。2026-09-05 的连接状态展示已能显示 Provider 在线，但 Runtime 卡片仍沿用单次查询流程，因此安装验证暴露了旧快照问题。

## 修复方案与涉及模块

- `src-tauri/src/runtime_gateway/tauri_bridge.rs` 与 `src-tauri/src/agent/runtime.rs`：初始化中的插件返回 `loading` 且无错误诊断；尚未首次启动的 generation 0 同样等待，停止/崩溃仍保留不可用诊断。
- `frontend/lib/providerRuntimes.ts`：主窗口统一订阅连接状态，先订阅再读取快照。Provider ID、连接状态或 generation 变化时重读 Runtime 列表，连续变化合并，旧请求结果失效。实例状态变化只更新 Harness 标签，不重复探测 executable。
- `frontend/App.svelte` 与 `ProviderConnectionStatus.svelte`：由同一份连接快照驱动卡片；状态组件只展示数据。Runtime 初始加载及手动操作都通过统一的列表刷新路径；启动中禁用依赖插件的操作并说明会自动检测。

## 排查顺序

先比对日志中的 Runtime 首次查询与 Provider 初始化完成时间，再看插件进程及心跳。若 Provider 已在线，可以点击“重新检测”验证是否是旧卡片结果。若仍失败，再查初始化失败日志、插件 stderr 和实际 Runtime API 错误；不要仅凭早期诊断重装 CLI 或重启 Harness。

## 验证结果

- `npx vitest run frontend`：196 项通过；覆盖启动后在线补拉、事件先于快照、旧库存晚返回、多次变化合并、普通实例更新不探测、离线及新 generation、订阅清理。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib runtime_gateway::tauri_bridge::tests`：6 项通过；真实延迟初始化 fixture 验证首次启动与初始化中为 loading，停止后仍有 `provider-unavailable`。
- Playwright 注入 Tauri 状态的页面验证通过：启动等待、在线补拉、离线诊断及恢复；检查焦点保持、操作可用性和 640px 下无卡片横向溢出。自动化不能替代安装版真实 IPC 验收。
- `npm run tauri -- build --bundles app,dmg --config <本地关闭 updater 产物配置>` 成功；Host 0.1.4 arm64 DMG 经 `hdiutil verify` 验证，应用经 `codesign --verify --deep --strict` 验证。使用本机 ad-hoc 签名，未公证。
- 全仓 `npx tsc --noEmit` 失败：未修改文件中缺少 Node 类型、旧测试 fixture 缺字段及已有可空值诊断。本次 `providerRuntimes.ts`、对应测试及 `agentRuntime.ts` 的独立严格类型检查通过。`git diff --check` 通过，未执行格式化工具。

## 回归防线与规约候选

遵守 `../60-rules/provider-dependent-ui-snapshots.md`：派生列表必须响应 Provider 生命周期，连接标签恢复不能代表依赖数据已恢复。每个异步回包都需要防止覆盖更新的状态。

## 未知项

真实安装版重新启动后的恢复仍需验收；本次证据不判断远程会话内容读取错误的独立原因。
