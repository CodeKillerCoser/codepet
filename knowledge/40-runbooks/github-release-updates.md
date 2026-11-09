# GitHub 构建与 Release 发布

## 入口与职责

- `.github/workflows/build.yml`（Build）：选择 `all/mac/win`，可选基础版本；同步版本、固定源码 commit，构建并上传 Actions 产物。不会创建 Release、tag 或更新 `latest.json`。
- `.github/workflows/release.yml`（Release）：输入一个成功的 Build 运行 ID，下载那次运行的产物、验证并发布。不会重新编译，不接受版本、平台或 tag 覆盖。
- `scripts/release_artifacts.mjs`：记录完整 commit、版本、所选平台和 run ID；规范安装包名称；校验发布输入完整且来自同一构建。
- `scripts/release_version.mjs`：基础版本同步和 `<base>+<short_commit>` 构建版本派生。
- `scripts/generate_latest_json.mjs`：依据实际签名产物生成 Windows updater 清单。

Windows 客户端通过 Tauri updater 请求固定 `latest.json`。macOS 从 GitHub Releases latest API 检查版本并匹配 DMG，提示手动安装；新规范名与旧版 Tauri 名称均兼容，策略位于 `src-tauri/src/platform/app_updates.rs`。旧客户端的 `macos-universal` 错误见 [排查记录](macos-update-target-missing.md)。

## 操作步骤

1. 在 Actions 的 **Build** 选择分支、平台，以及可选基础版本。也可以运行：

   ```sh
   npm run build:github -- --version 0.3.11-beta --platform all --ref main
   ```

2. 等待构建成功，从运行页直接下载 DMG 或 EXE 验证。记录 URL `/actions/runs/<ID>` 中的运行 ID。
3. 在 **Release** 填入 `build_run_id`，可选填写标题和说明。命令行等价入口：

   ```sh
   npm run release:github -- --build-run-id 123456789 --ref main
   ```

旧的 `release:github --version ...` 及 `publish` 开关已移除；发布必须指定已有 Build。此次拆分前的旧 Release 运行没有构建元数据，不能作为新发布入口的输入，需要先运行一次新 Build。

## 版本与产物名称

基础版本继续同步到 `package.json`、`package-lock.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 和 `src-tauri/Cargo.lock`。Build 的 preflight 在请求版本变化时提交并推送版本文件，再用该完整 commit 构建。构建版本是 `<base>+<commit前7位>`，tag 为 `v<构建版本>`，源码里不提交 SHA 后缀。

例如基础版本 `0.3.10-beta`、commit `a68ec0c...`：

| 产物 | 名称 |
| --- | --- |
| macOS ARM64 | `codepet-macos-arm64_0.3.10-beta_a68ec0c.dmg` |
| Windows x86_64 | `codepet-windows-x86_64_0.3.10-beta_a68ec0c-setup.exe` |
| Windows updater 签名 | `codepet-windows-x86_64_0.3.10-beta_a68ec0c-setup.exe.sig` |
| 构建元数据（双平台示例） | `codepet-build-all_0.3.10-beta_a68ec0c.json` |

平台选择为 mac/win 时元数据名称中的 all 相应替换。元数据包含完整 SHA、构建版本、tag、平台和 run ID。安装包只改外部文件名，不改签名后的内容；EXE 与 `.sig` 成对改名。

使用 [upload-artifact v7 的 `archive: false`](https://github.com/actions/upload-artifact/blob/v7/action.yml) 上传每个单文件；artifact 名称由真实文件名决定，不设置另一个展示名称。Mac 点击下载直接得到 DMG，不再套 ZIP，也不上传 `.app`。跨工作流读取使用 [download-artifact v8](https://github.com/actions/download-artifact)，校验摘要不一致即失败。

## 发布验证与失败重试

Release 要求来源为本仓库 `.github/workflows/build.yml` 的成功、已完成、手动触发运行。发布入口下载指定 run 的产物，验证元数据中的 run ID、版本与 commit 关系，以及所选平台的精确文件集合；缺包、空包、额外旧包、缺签名均失败。构建产物已过期时需重新 Build。

发布使用元数据中的精确 commit 作为 tag target；已有 tag 必须解析到相同 commit。先创建草稿，再上传安装包、构建元数据和 `latest.json`，全部成功才转正式 Release 并设为 latest。失败可用同一 Build 重试草稿；已正式发布的 tag 拒绝覆盖，请构建新版本。不会先删除既有公开安装包。

发布范围严格等于 Build 所选平台，不拼接其他运行的旧产物。`all` 构建有任一平台失败，即便另一平台已有下载产物，也不能发布。单平台发布的 latest 只含该平台当前产物；需要两平台一起升级时使用 `all`。

## 构建与签名约束

- macOS 固定 `aarch64-apple-darwin`，由 `scripts/macos_signing.py build` 导入固定证书，验证 App/Provider 签名，使用 `src-tauri/tauri.macos-dmg.conf.json` 只交付 ARM64 DMG。证书 Secrets 和验证方式见 [固定开发签名](macos-stable-development-signing.md)。不提供 Intel/universal 构建选项，不做 Apple notarization。
- Windows 固定 x86_64 NSIS，要求 `TAURI_SIGNING_PRIVATE_KEY`，可选 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。这是 updater 签名，不是系统代码签名。Tauri 构建保留三次重试。
- 两个平台先安装 Bun 1.4.2；`beforeBuildCommand` 运行 Provider staging。目标分别设为 `CODEPET_PROVIDER_TARGET=aarch64-apple-darwin` 和 `x86_64-pc-windows-msvc`；主 App、三个 Provider adapter 与 SDK 生成器必须一致。Provider adapter 不代替用户的 Agent runtime。
- 当前 `--mac-updater disabled`，清单仅写 Windows updater；Mac 单平台的 `platforms` 为空。DMG 不能充当 updater 归档，依据 [Tauri 文档](https://v2.tauri.app/plugin/updater/)，Mac 自动安装依赖签名 App tar.gz。旧生成脚本仍可显式 `--mac-updater enabled` 处理已有归档，当前 CI 不走此路径。
- `src-tauri/tauri.conf.json` updater endpoint 保持 `https://github.com/CodeKillerCoser/codepet/releases/latest/download/latest.json`；包 URL 指向具体 tag，不可把检查入口固定到旧 tag。
- Build 同一 ref 串行执行，防止版本同步冲突；Release 串行发布，防止同时修改 latest。GitHub API 不需要客户端登录；网络失败和匿名限流须显示检查失败，不伪装成无更新。

## 本地验证与风险对应

2026-11-09 拆分验证：

```sh
node --test scripts/generate_latest_json.test.mjs scripts/release_artifacts.test.mjs scripts/publish_github_release.test.mjs
actionlint .github/workflows/build.yml .github/workflows/release.yml
python3 scripts/macos_signing_test.py
npm run version:check
TAURI_CONFIG='{"app":{"macOSPrivateApi":false}}' cargo test --manifest-path src-tauri/Cargo.toml --lib platform::app_updates -- --nocapture
```

脚本测试覆盖双平台、两个单平台、文件字节不变、Windows 新 URL 和签名配对、缺包/空包/混包、版本及 run ID 不匹配、失败/取消/错误工作流拒绝发布，以及两个 CLI 的参数边界。Mac 测试同时检查旧文件名、新规范名、commit 和架构不匹配。

真实 CI 验收路径：Build all 成功 → 直接下载 DMG 并 `hdiutil verify`、App/Provider 验签与启动 → Release 指定同一 run → 比对 tag 完整 commit、发布文件名和大小、Windows 清单 URL → Mac/Windows 旧版本分别检查更新。取消自动更新提示后，手动检查仍需能提示。

## 未知项

本地模拟不能验证 GitHub 跨运行下载、Secrets 注入及最终发布；需按上面的真实 CI 验收路径检查，并以对应 Actions 运行结果为准。先前合并流程的成功不等于新流程验收完成。Windows 原生安装仍需 Windows 验收。

## Bun 缺失导致 staging 失败

- 证据：2026-09-07 的 [Release run 34085083215](https://github.com/CodeKillerCoser/codepet/actions/runs/34085083215) 构建提交 `1c4e33d`，macOS 和 Windows 都在 Rust release 编译成功后于 `buildSdkGeneratorBinary` 报 `spawnSync bun ENOENT`，三次重试结果相同。
- 根因：staging 依赖 Bun 编译 SDK 生成器，但 workflow 只安装 Node 和 Rust；本地已安装 Bun 的环境不能覆盖干净 runner 的依赖缺失。缺失的引入提交未确认。
- 修复：在两个构建 job 的 `npm ci` 前显式安装 Bun；保持现有统一 staging 入口和各目标平台编译参数。
- 本地验证：`npm run providers:test` 三项通过；用 staging 导出的 `resolveBunExecutable` 和 `sdkGeneratorBuildArguments` 实际编译 Windows `cp-sdk-gen.exe`，并执行 `--help`。macOS universal 合并和完整签名打包仍需在新提交的 Release workflow 中验证。
- 回归检查：遇到 `ENOENT` 先确认缺失的工具及 PATH，并检查所有构建 job 的安装步骤；重复构建不能修复缺少依赖。Bun 版本升级后重新验证 Windows executable 与 macOS 两种架构的编译、合并。
