# Mac 检查更新报 macos-universal 缺失

## 现象

2026-11-09，Mac 桌面端在“设置 → 关于 → 检查更新”报 `the platform \`macos-universal\` was not found in the response \`platforms\` object`。

## 证据

- 在本机 `0.3.9-beta+menubar.20261109.9` 原生界面复现了用户提供的完整错误。
- `v0.3.10-beta+a68ec0c` 的 `latest.json` 仅有 `windows-x86_64`，Release 提供 ARM64 DMG，没有 Mac updater 归档。
- 旧 `src-tauri/src/app/updates.rs` 在 macOS 固定调用 `.target("macos-universal")`。
- `published_manifest_reproduces_the_old_target_error` 用同形状清单调用当前 Tauri updater 的 `download_url`，复现相同错误。插件 2.10.1 的检查流程先读取平台下载地址，再返回版本比较结果，因此同版本也可能报错。

## 根因

发布产物与客户端交付策略不一致。单纯改成 ARM 平台键不能解决：清单没有任何 Mac updater 产物，DMG 也不能代替 Tauri 要求的签名 App tar.gz。

## 引入历史

已核对提交 `aec339b`（`ci: build ARM64-only macOS DMG artifacts`）：工作流切换为 ARM64 DMG-only、清单禁用 Mac updater，但没有同步修改客户端更新入口。

## 修复方案与涉及模块

- `platform/app_updates.rs`：统一检查和安装接口，Mac 从公开 GitHub latest Release 检查 SemVer，选择当前架构的已上传非空 DMG（兼容 universal），显式打开下载链接。下载地址限定在本项目仓库；Intel 不接收 ARM 包。Windows 保持 Tauri 自动安装。
- `app/updates.rs`：保存统一待更新对象；失败检查清理旧对象，打开下载失败保留对象以支持重试。
- `frontend/lib/types.ts`、`frontend/App.svelte`：新增可选 `manualDownload` 能力，显示“下载 DMG”及手动安装说明，避免误报自动安装；旧自动安装响应仍兼容。
- `frontend/qa/main-window.ts`、`scripts/app_updates_ui_qa.mjs`：用真实 App 界面和隔离 IPC fixture 检查两种交付流程。

## 验证结果

以下命令在本机通过：

```sh
TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo test --manifest-path src-tauri/Cargo.toml --lib platform::app_updates -- --nocapture
TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo test --manifest-path src-tauri/Cargo.toml --lib platform::app_updates::tests::live_dmg_check -- --ignored --nocapture
npx --no-install vitest run frontend/lib/updates.test.ts frontend/lib/api.test.ts
CODEPET_QA_NODE_MODULES=/tmp/codepet-pet-qa/node_modules node scripts/app_updates_ui_qa.mjs
```

Rust 6 项离线测试及 1 项显式联网测试通过；真实 API 返回新版本 ARM64 DMG，同版本再次检查返回无更新。前端 22 项测试及浏览器 QA 通过，覆盖下载、失败重试、取消后手动检查、Windows 自动安装按钮、无更新和网络错误。完整 Tauri release App 构建成功，版本为 `0.3.10-beta+updatefix.20261109.1`。

直接 Cargo 测试需上述配置覆盖；首次未覆盖时触发已有 Tauri feature/config 检查失败。Tauri CLI 构建自动添加的全局 `macos-private-api` 已恢复，保留 macOS target 配置。

## 回归防线与规约候选

发布策略变更必须同时验证旧版到新版、同版本检查与各平台产物匹配，见 [更新交付与发布产物一致性](../60-rules/update-delivery-artifacts.md)。缺包、下载失败或 API 错误必须可重试，不能当作“当前已是最新版本”。

## 未知项

- 本次未执行 Windows 原生安装，自动安装分支由界面 fixture 和保留的原生调用验证。
- 本地修复尚未发布，运行中的旧版未替换。原生逻辑变更需要完整安装新版，不能仅替换前端包。
- GitHub API 匿名限流及本机网络可能导致检查失败；界面会报告错误，不应隐瞒为无更新。
