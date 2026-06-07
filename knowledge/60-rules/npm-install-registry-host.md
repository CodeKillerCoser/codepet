# npm 安装 registry host 约束

## 规则

仓库如果提交 `package-lock.json`，必须避免 lockfile 中的 `resolved` tarball 指向当前环境不可访问或不稳定的旧 registry host。项目级 `.npmrc` 应声明可用 registry，并在需要时设置 `replace-registry-host=always`；镜像不支持 npm audit 接口时应关闭 `audit`。

## 适用场景

适用于前端依赖安装、CI 依赖恢复、Codex worktree 中执行 `npm i` / `npm ci`，以及任何会改写 `package-lock.json` 的依赖升级。

## 反例

`package-lock.json` 的大量 `resolved` 指向 `https://registry.anpm.alibaba-inc.com/`，但当前网络对该 host 下载 tarball 时持续 `ECONNRESET`。即使全局 npm registry 已设置为腾讯镜像，默认 `replace-registry-host=npmjs` 也不会替换这个旧 host，`npm i` 会长时间重试并可能触发 npm 自身的 `Exit handler never called`。

## 推荐做法

- 在项目 `.npmrc` 固定已验证可访问的 registry。
- 设置 `replace-registry-host=always`，让 npm 使用当前 registry 替换 lockfile 中的旧 host。
- 如果镜像 audit endpoint 返回 404，设置 `audit=false`。
- 更新 lockfile 时检查 `resolved` 是否仍包含旧 registry host。

## 来源

2026-06-07 排查 `npm i` 卡住：`npm i --timing --loglevel verbose` 显示大量 tarball 从 `registry.anpm.alibaba-inc.com` 下载时 `ECONNRESET`，而 `mirrors.cloud.tencent.com` 可正常返回。

## 验证方式

- `npm i`
- 使用空临时 cache 执行 `npm ci --cache <temp-cache> --prefer-online`
- `rg "registry\.anpm\.alibaba-inc\.com" package-lock.json` 应无结果
- `rg "mirrors\.cloud\.tencent\.com" package-lock.json .npmrc` 应无结果；2026-06-08 GitHub Actions 在 macOS 与 Windows runner 上曾因 `zimmerframe` 的腾讯镜像 tarball 404 导致 `npm ci` 失败。
