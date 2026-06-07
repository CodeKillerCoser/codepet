# GitHub Release 更新发布

## 现象

需要发布 Code Pet 新版本时，客户端会通过 Tauri updater 请求 GitHub Release 中的 `latest.json`，再按当前平台下载对应 updater 产物。

## 数据来源

- `src-tauri/tauri.conf.json`：updater 公钥、endpoint 和 `createUpdaterArtifacts`。
- `scripts/generate_latest_json.mjs`：从 Release 产物和 `.sig` 文件生成 `latest.json`。
- `.github/workflows/release.yml`：手动触发 macOS universal 与 Windows x86_64 构建，并创建或更新 GitHub Release。
- GitHub Release asset：客户端直接下载的安装包、updater 包、签名文件和 `latest.json`。

## 发布前检查

- 仓库需要配置 secret `TAURI_SIGNING_PRIVATE_KEY`。如果私钥有密码，同时配置 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。
- 这两个 secret 是 Tauri updater 签名，不是 Apple/Windows 代码签名。当前 workflow 不做 notarization、Apple 签名或 Windows 签名。
- workflow 会在构建前检查 `TAURI_SIGNING_PRIVATE_KEY` 非空；Tauri build 需要通过这个环境变量读取 updater 私钥。
- 当前 `latest.json` 只发布 `macos-universal` 和 `windows-x86_64` 两个平台键。
- 公共 GitHub Release asset 可匿名下载，客户端检查更新不需要 GitHub 身份验证。私有仓库或私有 Release 不适合当前静态 endpoint 方案。

## 发布步骤

在本地有 GitHub CLI 且已登录时，可运行：

```powershell
npm run release:github -- --tag v0.1.0 --ref main
```

也可以直接在 GitHub Actions 页面手动触发 `Release` workflow。未填写 tag 时，workflow 使用 `src-tauri/tauri.conf.json` 里的版本生成 `v<version>`。

workflow 完成后，Release 应包含：

- macOS `.dmg`。
- macOS `.app.tar.gz` 与 `.app.tar.gz.sig`。
- Windows `*setup.exe` 与 `*setup.exe.sig`。
- `latest.json`。

macOS workflow 需要使用 `--bundles app,dmg`。只构建 `dmg` 时 Release 里会有安装包，但不会生成 updater 使用的 `.app.tar.gz`。

## 验证

- 本地语法检查：`node --check scripts/generate_latest_json.mjs` 和 `node --check scripts/publish_github_release.mjs`。
- 本地 manifest 生成：用临时目录放置 `.app.tar.gz`、`.app.tar.gz.sig`、`*setup.exe`、`*setup.exe.sig`，运行 `npm run release:latest-json -- --repo CodeKillerCoser/codepet --tag v0.1.0 --artifacts <dir> --output <file>`。
- Release 后检查 `https://github.com/CodeKillerCoser/codepet/releases/latest/download/latest.json` 可公开访问，且 JSON 中两个 URL 都能下载。
- 客户端验证：安装旧版本后手动检查更新，确认弹窗出现；取消后自动检查不再提示同一版本，手动检查仍会提示。

## 风险

- `latest.json` 使用 `releases/latest` endpoint。如果 GitHub latest 指向了错误 Release，客户端会读取错误版本。发布 job 会用 `gh release ... --latest` 标记当前 Release。
- Windows 当前是 x86_64 runner 产物。如果未来需要 32-bit x86，需要增加新的构建 target 和平台键。
- 未签名或未公证的安装包可能触发系统安全提示，这不影响 updater manifest，但会影响用户安装体验。

## 未知项

- 尚未在 GitHub Actions 真实 runner 上完成一次端到端 Release 构建。
