# GitHub Release 更新发布

## 现象

需要发布 Code Pet 新版本时，客户端会通过 Tauri updater 请求 GitHub Release 中的 `latest.json`，再按当前平台下载对应 updater 产物。

## 数据来源

- `src-tauri/tauri.conf.json`：updater 公钥、endpoint 和 `createUpdaterArtifacts`。
- `scripts/release_version.mjs`：读写基础版本，校验 `package.json`、`package-lock.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 和 `src-tauri/Cargo.lock`，并能派生 `<base>+<short_commit>` 构建版本。
- `scripts/generate_latest_json.mjs`：从 Release 产物和 `.sig` 文件生成 `latest.json`。
- `scripts/stage_provider_plugins.mjs`：为目标平台构建 Codex/OpenCode/Claude 三个 Code Pet Provider adapter，并 staging 对应 manifest 与可执行文件。
- `.github/workflows/release.yml`：手动触发 macOS universal 与 Windows x86_64 构建，并创建或更新 GitHub Release。
- GitHub Release asset：客户端直接下载的安装包、updater 包、签名文件和 `latest.json`。

## 发布前检查

- 仓库需要配置 secret `TAURI_SIGNING_PRIVATE_KEY`。如果私钥有密码，同时配置 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。
- 这两个 secret 是 Tauri updater 签名，不是 Apple/Windows 代码签名。当前 workflow 不做 notarization、Apple 签名或 Windows 签名。
- workflow 会在构建前检查 `TAURI_SIGNING_PRIVATE_KEY` 非空；Tauri build 需要通过这个环境变量读取 updater 私钥。
- 当前 `latest.json` 只发布 `macos-universal` 和 `windows-x86_64` 两个平台键。
- Tauri `beforeBuildCommand` 会先运行 `npm run providers:stage`。macOS job 必须设置 `CODEPET_PROVIDER_TARGET=universal-apple-darwin`，Windows job 必须设置 `CODEPET_PROVIDER_TARGET=x86_64-pc-windows-msvc`；不要只把 App 主程序构建成 universal。
- 两个构建 job 都必须先通过 `oven-sh/setup-bun@v2` 安装 Bun（当前固定 `1.4.2`）。`providers:stage` 除 Rust Provider 外还调用 `bun build --compile` 生成 `cp-sdk-gen`；`npm ci` 不会安装此构建工具。调整 staging 工具依赖时，必须同步检查两个平台的 runner 初始化步骤。
- 三个内置二进制是 Code Pet Provider adapter，不是 Codex、OpenCode 或 Claude runtime。安装包不会替用户安装或选择底层 runtime，本机 `AgentRuntimeService` 检测与用户配置仍是权威。
- `src-tauri/tauri.conf.json` 中的 updater endpoint 必须是固定检查入口 `https://github.com/CodeKillerCoser/codepet/releases/latest/download/latest.json`。不要配置成 `releases/download/<tag>/latest.json`，否则旧客户端会一直检查旧 tag 下的 manifest，无法发现新版本。
- 仓库提交的版本必须是基础版本，例如 `0.1.4`，不要提交 `0.1.4+<sha>`。构建版本由 workflow 基于版本同步后的 commit 派生。
- 发布新版本时优先在 workflow 的 `version` 输入中填写基础版本，或通过本地 `npm run release:github -- --version <version> --ref main` 触发。`preflight` job 会先把基础版本同步并推送回当前分支，再输出 `version=<base>+<short_commit>` 给 macOS/Windows 构建。
- Release workflow 会先运行 `preflight` job。基础版本格式错误、版本源不一致、无法同步分支，或手动 tag 与最终构建版本不一致时，会在 macOS/Windows 构建开始前失败。手动 tag 若只是 `v<base>` 或 `<base>`，会按留空处理并自动派生最终 tag。
- 从 `v0` 等非默认分支发布时，`gh release create` 必须显式传入 `--target`，值为 preflight 的完整 `commit_sha`。否则尚不存在的 tag 会落在默认分支，导致安装包来自所选分支而 Release 源码指向其他提交。2026-09-06 发布前检查发现并修正此遗漏；验收时比对远端 tag 与 preflight commit。
- 公共 GitHub Release asset 可匿名下载，客户端检查更新不需要 GitHub 身份验证。私有仓库或私有 Release 不适合当前静态 endpoint 方案。

## 发布步骤

`platform` 可选 `all`（默认）、`mac`、`win`，分别构建全部平台、macOS universal、Windows x86_64。单平台发布只包含该平台资产与 `latest.json` 平台键；不会沿用另一平台旧版本的资产。发布仍要求所选构建成功，失败或取消不能发布。若要让两个平台都能从 latest 入口获取本次更新，选择 `all`。

本地发布命令也支持 `--platform mac` 或 `--platform win`，省略时为 `all`，例如 `npm run release:github -- --ref v0 --platform mac`。

GitHub Actions 的 `Release` workflow 增加布尔输入 `publish`，默认不勾选：所选平台正常构建并上传 Actions Artifacts，跳过整个发布 job，不创建或更新 Release、tag 和 `latest.json`。正式发布时勾选 `publish`。本地 `npm run release:github` 命令显式传入 `publish=true`，保持发布语义。

只构建时若填写 `version`，仍会同步并提交基础版本到所选分支；构建仍需 updater 签名 secret。验收时分别运行未勾选和勾选的 workflow：前者应有所选平台 Artifacts 且发布 job skipped，后者应生成下述 Release 资产。

在本地有 GitHub CLI 且已登录时，可运行：

```powershell
npm run version:check
npm run release:github -- --version 0.1.4 --ref main
```

也可以直接在 GitHub Actions 页面手动触发 `Release` workflow。日常发布填写 `version`，不要填写 `tag`；workflow 会同步基础版本并生成 `v<version+short_commit>`。如果误填 `v<version>` 或 `<version>`，workflow 会按留空处理。如果填写其他 tag，`preflight` 会校验 tag 去掉可选 `v` 前缀后必须等于最终构建版本。

如果只是本地提升仓库基础版本，可以运行：

```powershell
npm run version:sync -- 0.1.4
npm run version:check
```

选择 `all` 并发布后，Release 应包含以下全部文件；单平台时只包含对应平台文件及 `latest.json`：

- macOS `.dmg`。
- macOS `.app.tar.gz` 与 `.app.tar.gz.sig`。
- Windows `*setup.exe` 与 `*setup.exe.sig`。
- `latest.json`。

更新已有 release 时，workflow 会先删除该 release 的旧资产再上传本次构建产物，避免同一个 tag 下同时残留旧版本和新版本安装包。

macOS workflow 需要使用 `--bundles app,dmg`。只构建 `dmg` 时 Release 里会有安装包，但不会生成 updater 使用的 `.app.tar.gz`。

本地只验证 staging 时可运行：

```bash
npm run providers:test
CODEPET_PROVIDER_TARGET=universal-apple-darwin npm run providers:stage
```

开发态可把 `CODEPET_BUNDLED_PROVIDER_PLUGINS_DIR` 设置为 `src-tauri/resources/provider-plugins` 的绝对路径；发行态不设置该变量，App 使用 Tauri resource API 从自身 Resources 的 `provider-plugins/` 定位。

GitHub Release 上传资产时会把文件名中的空格规范化为 `.`，`latest.json` 里的 URL 必须使用规范化后的资产名。

`latest.json` 的检查入口是固定的 `releases/latest/download/latest.json`；JSON 内部的平台安装包 URL 可以、也应该指向当前版本的具体 tag asset。检查更新读取固定入口，下载安装时再使用 manifest 中的版本化资产 URL。

## 验证

- 本地语法检查：`node --check scripts/release_version.mjs`、`node --check scripts/generate_latest_json.mjs` 和 `node --check scripts/publish_github_release.mjs`。
- 本地版本一致性检查：`npm run version:check`。
- Provider staging 检查：`npm run providers:test`，确认三 manifest、三平台 executable、Unix 可执行位和 Windows `.exe` manifest。
- macOS App 检查：确认 `Contents/Resources/provider-plugins/{codex,opencode,claude}/` 各有 `codepet-provider.json` 与同名 executable；对 `Contents/MacOS/code-pet` 和三个 Provider 分别运行 `lipo -archs`，都必须同时包含 `x86_64 arm64`。
- macOS DMG 检查：运行 `hdiutil verify <dmg>`，并对 App/DMG 记录绝对路径、字节大小和 `shasum -a 256`。
- Windows workflow 检查：staging manifest 的 executable 必须带 `.exe`，NSIS 继续从同一个 `provider-plugins/` resources 映射收录文件；本机 macOS 不要求交叉生成安装包。
- 本地基础版本同步检查：`npm run version:sync -- <version>` 后运行 `npm run version:check`。
- 本地 manifest 生成：用临时目录放置 `.app.tar.gz`、`.app.tar.gz.sig`、`*setup.exe`、`*setup.exe.sig`，运行 `npm run release:latest-json -- --repo CodeKillerCoser/codepet --tag v0.1.4+2e4ca34 --version 0.1.4+2e4ca34 --artifacts <dir> --output <file>`。
- manifest 生成脚本会校验 `src-tauri/tauri.conf.json` 的 updater endpoint 是否等于固定检查入口。该检查失败时，应优先修正 `tauri.conf.json`，不要改脚本绕过。
- manifest 生成脚本会校验 release tag 与传入的构建版本一致。该检查失败时，应先确认 `preflight` 输出的 `version` 和 `tag`，不要手动发布 tag 是新版本但 `latest.json.version` 仍是旧版本的 release。
- Release 后检查 `https://github.com/CodeKillerCoser/codepet/releases/latest/download/latest.json` 可公开访问，且 JSON 中两个 URL 都能下载。
- 客户端验证：安装旧版本后手动检查更新，确认弹窗出现；取消后自动检查不再提示同一版本，手动检查仍会提示。

## Bun 缺失导致 staging 失败

- 证据：2026-09-07 的 [Release run 34085083215](https://github.com/CodeKillerCoser/codepet/actions/runs/34085083215) 构建提交 `1c4e33d`，macOS 和 Windows 都在 Rust release 编译成功后于 `buildSdkGeneratorBinary` 报 `spawnSync bun ENOENT`，三次重试结果相同。
- 根因：staging 依赖 Bun 编译 SDK 生成器，但 workflow 只安装 Node 和 Rust；本地已安装 Bun 的环境不能覆盖干净 runner 的依赖缺失。缺失的引入提交未确认。
- 修复：在两个构建 job 的 `npm ci` 前显式安装 Bun；保持现有统一 staging 入口和各目标平台编译参数。
- 本地验证：`npm run providers:test` 三项通过；用 staging 导出的 `resolveBunExecutable` 和 `sdkGeneratorBuildArguments` 实际编译 Windows `cp-sdk-gen.exe`，并执行 `--help`。macOS universal 合并和完整签名打包仍需在新提交的 Release workflow 中验证。
- 回归检查：遇到 `ENOENT` 先确认缺失的工具及 PATH，并检查所有构建 job 的安装步骤；重复构建不能修复缺少依赖。Bun 版本升级后重新验证 Windows executable 与 macOS 两种架构的编译、合并。

## 风险

- `latest.json` 使用 `releases/latest` endpoint。如果 GitHub latest 指向了错误 Release，客户端会读取错误版本。发布 job 会用 `gh release ... --latest` 标记当前 Release。
- Windows 当前是 x86_64 runner 产物。如果未来需要 32-bit x86，需要增加新的构建 target 和平台键。
- 未签名或未公证的安装包可能触发系统安全提示，这不影响 updater manifest，但会影响用户安装体验。
- 缺少 `TAURI_SIGNING_PRIVATE_KEY` 时，Tauri 可能在 App/DMG 已经生成后因 updater artifact 签名返回非零。排查时先检查 bundle 目录并验证既有 App/DMG，再把“bundle 产物存在”和“updater 签名失败”分别报告，不能把后者误报为未构建。
- Tauri 打包阶段会下载 NSIS 等外部工具，GitHub 下载链路偶发 `504` 时可能导致 Windows build 失败。workflow 对 Tauri build 做三次重试；如果三次都失败，应先按 job 日志确认是否仍是外部下载错误。

## 未知项

- 尚未在 GitHub Actions 真实 runner 上完成一次端到端 Release 构建。
- 构建版本使用 SemVer build metadata，即 `+<short_commit>`。SemVer 比较会忽略 build metadata，因此同一基础版本下不同 commit 后缀不应作为强制升级手段；需要强制升级时必须提升基础版本。
