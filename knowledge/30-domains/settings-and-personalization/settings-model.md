# 设置模型

## 当前模型

`src-tauri/src/app/settings.rs` 中的 `AppSettings` 顶层区域包括：

- `data`
- `connection`
- `appearance`
- `pet`
- `petLibrary`
- `notifications`
- `activityFilters`
- `agents`
- `agentRuntimes`
- `providerPlugins`
- `updates`

所有区域都使用 serde defaults，确保新增字段后旧设置文件仍能加载。

`activityFilters` 现在以 `byAgent` 保存每个 Agent 的标题和内容关键词过滤。旧的顶层 `titleKeywords` 和 `messageKeywords` 字段保留为兼容入口；前端归一化会把旧全局过滤迁移成每个 Agent 各自的过滤配置。

`agents.byAgent.<agent>.hookEvents` 保存每个 Agent 勾选的 hook 事件。缺失或空列表表示默认使用该 Agent 注册表中的全部支持事件。

`agentRuntimes.byProvider.<providerPluginId>.configuredExecutable` 保存对应 Provider 通过 `runtime.select` 验证并返回的 Runtime 路径；旧的 `codex`、`claude`、`opencode` key 只在读取时迁移映射到内置 pluginId。Host 不验证路径或产品版本。通用 settings 更新会保留后端当前值，`runtime.getInstalled` 的安装清单是 Provider 即时探测结果，不写入 settings。

`providerPlugins.directories` 保存额外 Provider plugin manifest 目录。缺失时默认空数组；Host 另有应用数据目录下的固定 `provider-plugins/` 默认目录。该字段只配置 Plugin Catalog，不改变 `agentRuntimes` 的 executable 探测，也不授权桌宠动作调用 Plugin Manager。

## 系统权限入口

设置导航的“权限”页通过 `frontend/lib/permissions.ts` 调用统一的原生权限接口，`src-tauri/src/platform/permissions.rs` 负责 macOS 设置跳转与其他平台的不可用反馈，不向 `AppSettings` 写入授权状态。

macOS 页面包含完全磁盘访问的用途、授权步骤、“前往授权”和“在访达中显示 App”。后者定位当前运行的 `.app`，不硬编码 `/Applications`；开发态未打包时显示安装提示。两个操作仅允许主窗口调用，目标由原生代码固定，不接受任意路径或 URL。

打开系统设置不代表权限获批，界面始终以 macOS 系统设置为权威；不读取 TCC 数据库、不探测用户的敏感文件、不自行修改权限。用户应在系统设置开启 Code Pet，退出重启后重试。非 macOS 平台显示不适用说明；旧后端或跳转失败显示可恢复错误。

验证路径：原生单测覆盖实际 bundle 路径解析和能力返回；前端构建及导航回归；通过 `frontend/qa/main-window.html` 检查布局、键盘焦点和按钮反馈，附加 `permissionUnsupported` / `permissionLoadError` / `permissionOpenError` query 验证平台与错误分支。真实 App 验证系统页面跳转及访达选中，不自动开启权限。

2026-11-07 已运行 `npm run build`；`npx vitest run frontend` 通过 207 项，随后新增旧后端错误恢复场景，权限组件 4 项测试单独通过（包含焦点保持、防重复请求、失败重试、非 macOS 分支）。`vite.config.ts` 仅在 Vitest 中选择浏览器条件，确保 jsdom 执行 Svelte 客户端生命周期；开发和生产构建沿用原解析行为。

原生验证使用仓库既定的 `TAURI_CONFIG='{"app":{"macOSPrivateApi":false},"bundle":{"resources":[]}}' cargo test --manifest-path src-tauri/Cargo.toml --lib`，88 项通过，其中权限模块 2 项。裸 Cargo 命令触发已有的 macOS private API 配置/feature 校验冲突，未为此次测试修改产品配置。电脑控制工具返回 `cgWindowNotFound`，浏览器 host 不可用，因此真实布局、窗口尺寸及系统跳转尚待桌面可访问后验收；组件测试不替代视觉验证。

## 前端归一化

`frontend/App.svelte` 会在使用加载到的设置前做归一化：

- 运行中气泡默认值和数值边界。
- 应用数据目录缺省对象。
- 图片像素尺寸。
- 宠物透明度。
- 抽打反应音默认值。
- activity filter 关键词按 Agent 去空白和去重。
- agent hook 事件按注册表顺序过滤非法值，默认全选。
- Provider plugin 目录缺失时归一化为空数组。

## 风险

新增字段如果没有 Rust 默认值或前端归一化，可能破坏旧用户设置，或产生 `undefined` UI 状态。数据目录覆盖项必须保留系统 local data 下的固定 settings 入口，否则应用启动时无法先定位配置再定位自定义数据目录。

Agent runtime 配置失效时必须保留原值并展示诊断，不能静默写入自动检测结果。用户显式“恢复自动检测”后才清除配置。

数据目录变更属于迁移操作：保存新配置前应复制旧目录内容到目标目录、跳过固定 `settings.json`，并拒绝新旧数据目录互相包含，避免递归复制或清空源数据。用户选定的自定义目录必须为空；非空目录必须先由前端弹窗确认，后端收到确认标记后才允许清空目标目录并继续迁移。前端需要提示保存完成后重启，后端只保证迁移和设置落盘。

按 Agent 过滤时，不能再把顶层 `activityFilters.titleKeywords/messageKeywords` 当成新配置写回；否则会重新变成全局过滤。

## 验证

- 运行 `cargo test --manifest-path src-tauri/Cargo.toml settings_tests`。
- 默认值影响 UI helper 时，新增或更新前端测试。
