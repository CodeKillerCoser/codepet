# Release 版本源

## 规则

发布版本以 `src-tauri/tauri.conf.json.version` 为唯一应用版本源。Release tag 只能由该版本派生，不能反向驱动构建版本。

## 适用场景

修改 GitHub Release workflow、updater manifest 生成脚本、本地发布脚本或应用版本号时适用。

## 反例

在 GitHub Actions 中手动填写 `v0.1.2`，但 `tauri.conf.json` 仍是 `0.1.1`。这会让构建产物版本和发布 tag 不一致，并可能让 updater manifest 指向错误版本。

## 推荐做法

- 发布前运行 `npm run version:check`。
- 日常发布不填写 workflow 的 tag 输入，让 workflow 从源码版本生成 `v<version>`。
- 如果确实填写 tag，必须让 `tag.replace(/^v/, "")` 等于 `src-tauri/tauri.conf.json.version`。
- 修改版本时同时更新 `package.json`、`package-lock.json`、`src-tauri/Cargo.toml` 和 `src-tauri/Cargo.lock`。

## 来源

Release 构建曾反复因 `tauri.conf.json` 中定义的版本与发布时指定的版本不一致而失败。当前回归防线是 `.github/workflows/release.yml` 的 `preflight` job 和 `scripts/release_version.mjs`。

## 验证方式

- `npm run version:check`
- `node --check scripts/release_version.mjs`
- Release workflow 的 `preflight` job 必须在 macOS/Windows 构建 job 前运行。
