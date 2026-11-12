# 独立 Provider 无需 Host 选择运行时

## 现象与证据

2026-11-11，四个只依赖公开合同开发的插件经原生入口导入后，Provider 在线、实例 Ready，但卡片显示“未选择运行时”和未找到兼容运行时的诊断。独立 Echo/Pi 的 `runtime.getInstalled` 返回空 harnessList、selected:null。真实请求与 pi/DeepSeek 调用已通过，不能把空清单直接当成插件不可用。

## 根因与边界

桌面桥接将所有完成的空清单解释成检测失败。空清单既可能代表确实没有找到 Harness，也可能代表插件自行管理依赖；原协议没有表达这一区别。引入历史未确认。本次不根据名称、语言、Ready 状态或空清单推测依赖，也不改变 manifest command 的进程启动职责。

## 修复

Provider `RuntimeGetInstalledResponse` 增加可选 `selectionRequired`。缺省或 true 沿用旧检测行为；false 明确表示插件管理自身依赖，无需 Host 选择。此时必须返回空清单、selected:null、scanning:false 或缺省，且无 scanError。RPC 与 runtime.inventoryChanged 通知共用语义验证，矛盾声明被拒绝；不以 false 掩盖错误。切为 false 时不保留旧 Harness 选择。

桌面派生为 not-required，保留 Provider 在线状态和实例状态，路径/版本选择控件替换为“由 Provider 管理，无需选择”。这只说明选择职责，不保证 Node、pi CLI 或其他依赖可用；实际启动失败仍由实例诊断表达。内置 Provider 不声明此字段，原来的扫描与卸载提示保持。

## 涉及模块与验证

- canonical Provider schema、生成的 Rust/TS DTO 和公开指南：添加可选合同；执行 freshness、协议正负例、导出闭合检查及生成器测试。
- Host runtime_inventory/manager：RPC 与通知验证、缓存合并；单测比较缺省和 false，拒绝扫描/错误矛盾；manager_gateway 回归旧扫描、取消和隔离行为。
- Tauri runtime_gateway、AgentRuntimeStatus、前端类型和组件：状态映射和控件；真实子进程桥接测试先确认旧空清单仍为 runtime-not-found，再确认 false 无错误；前端状态测试与构建、原生卡片验收。
- 独立示例仍由仅看到新版导出合同的子 agent 更新并打包，父 agent 不修改插件实现。原生更新后检查独立卡片，并保留内置路径选择。

## 验收记录

2026-11-11 已完成：Host 新增语义单测 1 项通过；manager_gateway 46 项通过；外部三 profile 互通/半包隔离 2 项通过，并单独通过三 profile 新字段断言；桌面桥接 7 项、前端状态/观察器 11 项、协议及导出 25 项、生成器 3 项均通过。独立作者源码及四个 bundle 各 13 项通过，未调用付费模型。协议 freshness 和 git diff --check 通过。

桥接使用 `TAURI_CONFIG='{"app":{"macOSPrivateApi":true}}' cargo test --manifest-path src-tauri/Cargo.toml --lib runtime_gateway::tauri_bridge::tests`；该配置与 Tauri CLI 构建期间的 macOS feature 一致。早期一次 false 配置与 CLI 增加的 feature 不一致而被构建校验拒绝，后按一致配置通过，最终恢复 CLI 对 Cargo.toml 的额外修改。

`npm run tauri -- build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'` 构建通过。本机安装新版并更新四个独立包（保留各自 manifest 启动配置），原生卡片实际确认四者在线、实例可用且显示“由 Provider 管理，无需选择”，旧版 Pi 和内置 Provider 仍保留路径选择。安装 App 的 SHA256 为 `79fa8e3c080e9ed0a9e804cc41ffb30236317af94866a2ad1d71b3ea604602a2`，SDK 已包含新字段。插件 bundle 与作者交付逐一校验一致。旧 App/插件分别备份于 `/tmp/codepet-before-runtime-selection.app`、`/tmp/codepet-providers-before-runtime-selection`。

Windows/Linux 原生界面和额外窗口尺寸未实机验证；不宣称运行时依赖自动安装或新 Observation-only 合同已经完成。
