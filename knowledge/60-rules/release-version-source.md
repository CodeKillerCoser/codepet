# Release 版本源

## 规则

仓库中提交的应用版本必须是基础 SemVer，例如 `0.1.4`。Release workflow 可以接收 `version` 输入，并先把这个基础版本同步回分支；构建产物版本由 CI 在工作区派生为 `<base>+<short_commit>`，例如 `0.1.4+2e4ca34`。

不要把带 commit 后缀的构建版本直接提交到仓库。commit id 来自版本同步后的 commit；如果把 commit id 写进同一个 commit，会形成自引用，导致版本号与真实 commit 无法稳定一致。

## 适用场景

修改 GitHub Release workflow、updater manifest 生成脚本、本地发布脚本或应用版本号时适用。

## 反例

在 GitHub Actions 中手动填写 `v0.1.2`，但 `tauri.conf.json` 仍是 `0.1.1`。这会让构建产物版本和发布 tag 不一致，并可能让 updater manifest 指向错误版本。

另一个反例是把 `0.1.4+<current_commit>` 提交到仓库，并要求 `<current_commit>` 等于该提交自身。版本文件内容会改变 commit hash，因此无法同时满足。

## 推荐做法

- 发布前运行 `npm run version:check`。
- 本地同步基础版本时运行 `npm run version:sync -- <version>`，例如 `npm run version:sync -- 0.1.4`。
- 日常发布通过 workflow 的 `version` 输入或 `npm run release:github -- --version <version>` 指定基础版本，让 CI 负责同步仓库版本并派生 `v<version+short_commit>`。
- 日常发布不填写 workflow 的 `tag` 输入。只有明确知道最终构建版本时才填写 tag，且 `tag.replace(/^v/, "")` 必须等于 CI 派生出的构建版本。
- 修改基础版本时同时更新 `package.json`、`package-lock.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 和 `src-tauri/Cargo.lock`；应优先使用版本同步脚本完成。

## 来源

Release 构建曾反复因 `tauri.conf.json` 中定义的版本与发布时指定的版本不一致而失败。当前回归防线是 `.github/workflows/release.yml` 的 `preflight` job 和 `scripts/release_version.mjs`。后续要求构建版本带 commit 后缀时，必须保留“基础版本提交、构建版本派生”的边界。

## 验证方式

- `npm run version:check`
- `npm run version:sync -- <version>`
- `node --check scripts/release_version.mjs`
- Release workflow 的 `preflight` job 必须在 macOS/Windows 构建 job 前运行，并输出 `version=<base>+<short_commit>` 与 `commit_sha`。
