# macOS 固定自签名开发证书与 CI 签名

## 背景与目标

Code Pet 默认 `signingIdentity: "-"` 为 ad-hoc 签名，代码变化可能改变系统用于识别应用的 designated requirement。开发分发需要复用同一张非 Apple 代码签名证书，保持证书身份与 `com.codepet.desktop` 标识一致。证据见 [文件授权排查](macos-file-access-authorization.md)。

本方案不提供 Apple 公证或 Gatekeeper 信任，不自动授予文件权限。首次从 ad-hoc 切换证书身份可能需要重新授权；后续更新的 TCC 保留必须实测。

## 实现与涉及模块

- `scripts/macos_signing.py generate`：一次性生成 RSA 3072 / SHA-256、有效期 3650 天的自签名代码签名证书，导出加密 PKCS#12。目录权限 0700，文件 0600，拒绝写入仓库或覆盖已有目录，防止误提交私钥或每次构建换身份。
- `.github/workflows/release.yml` 的 macOS job：通过三个 Secret 向 `macos_signing.py build` 提供固定身份。Windows updater 签名和本地默认 ad-hoc 配置保持原流程。
- `build`：只接受 GitHub 托管 macOS runner；导入前校验公有证书指纹和至少 30 天剩余有效期。临时 keychain 只为 codesign 开放私钥；自签名证书在临时 runner 上仅为 codeSign 策略设置信任，随后用指纹覆盖 `APPLE_SIGNING_IDENTITY`。Tauri 子进程不接收证书 Secret，也不沿用其他 Apple 账号/证书环境变量。
- 验收：先签两个不同测试二进制，比对证书和 designated requirement 一致且不依赖 cdhash；再以 `--bundles app,dmg` 保留 App 并验证深度签名、证书指纹和应用标识。只发布 DMG，macOS updater 仍关闭。每条签名辅助命令限时 120 秒；凭据操作日志只输出工具名和操作名，验签失败可显示公有诊断。成功或异常退出均尝试恢复 keychain 列表、删除包含私钥的临时 keychain；codeSign 信任记录随一次性 runner 销毁，不调用可能进入交互等待的 `remove-trusted-cert`。清理错误不覆盖原始业务错误。
- `scripts/macos_signing_test.py`：覆盖凭据缺失/格式错误、指纹不匹配、拒绝本机/自托管 runner、异常清理、错误不回显密码、真实 PKCS#12 生成解析、拒绝覆盖和拒绝仓库内生成。

## 一次性配置

本轮已在本机生成 `~/.config/codepet/signing/development/`，证书名称 `Code Pet Development Code Signing`，指纹 `5FA53F2C971BE12909E9B2C9CFCC86E63632A590`。目录包含 `identity.p12`、`password.txt`、`certificate.pem`、`fingerprint.txt`；请整体备份到合适的凭据存储。公有指纹可记录，私钥、密码及 p12 base64 不进入仓库或日志。

仅在首次创建新身份时执行：

```sh
python3 scripts/macos_signing.py generate --output ~/.config/codepet/signing/development
```

为 `CodeKillerCoser/codepet` 配置 Repository Actions Secrets：

| Secret | 来源 |
| --- | --- |
| `CODEPET_MACOS_CERTIFICATE_BASE64` | `identity.p12` 的单行 base64 |
| `CODEPET_MACOS_CERTIFICATE_PASSWORD` | `password.txt` 内容 |
| `CODEPET_MACOS_CERTIFICATE_SHA1` | `fingerprint.txt` 内容 |

上传必须用文件/标准输入传递给 `gh secret set --repo CodeKillerCoser/codepet`，不将私钥、密码拼入命令或粘进聊天。证书 Secret 与 Tauri updater 的 `TAURI_SIGNING_PRIVATE_KEY` 相互独立。不能在 workflow 内运行 `generate`；证书到期或丢失时应显式规划身份迁移。

2026-11-07 已创建本机证书，并通过 `gh secret set` 的标准输入写入上述三个 Repository Actions Secrets；随后查询名称与更新时间确认存在。没有替换其他项目的证书或原有 updater Secret。用户随后要求将流水线切换为此证书并执行签名构建；真实 CI 验证结果在完成后记录于本文。

## 风险与验证计划

- 配错/换掉证书：生成器拒绝覆盖；CI 对照固定指纹、产物证书与 DR。改指纹或替换证书时必须按身份迁移处理。
- 私钥泄露：生成文件仅当前用户可读且位于仓库外；脚本错误隐藏命令参数和输出，构建子进程清除证书 Secret。检查 Git diff 和 Actions 日志只含公有指纹。
- 信任设置遗留：仅一次性托管 runner 执行导入；异常测试检查 keychain 列表恢复和私钥清理，信任记录随 VM 销毁。实际 CI 检查 cleanup 成功，不支持用本机伪造 runner 环境运行。
- 自签名不被构建工具接受：CI 的双二进制 smoke 在完整构建前失败；上线验收需确认 smoke、App 验签和 DMG 产物上传均成功。
- TCC 授权仍重复：用同一证书的两个不同版本，在测试 Mac 上授权版本 A 的 Documents/Downloads，替换版本 B、退出重启，再经 Code Pet 自身访问相同目录。记录弹窗应用名、版本、DR；当前终端能访问不构成验证。

## 本轮验证与未知项

`python3.12 scripts/macos_signing_test.py`（11 项）、`python3.12 scripts/package_signed_test.py`（6 项）、`node --test scripts/generate_latest_json.test.mjs`（4 项）、`actionlint .github/workflows/release.yml`、`git diff --check` 均通过。生成了真实证书并解析验证加密 p12，未修改本机 keychain/信任设置。完整 CI 签名和下载后的 DMG/App 验签已通过；安装启动行为和 TCC 跨版本保留仍未实测。

首次真实运行 [42373659047](https://github.com/CodeKillerCoser/codepet/actions/runs/42373659047) 的 Windows 构建成功并上传产物；macOS 证书导入成功，但预检未结束，取消后取日志。第二次运行 [42375134117](https://github.com/CodeKillerCoser/codepet/actions/runs/42375134117) 加入限时和阶段日志，确认测试二进制的签名与深度验签已成功，证书提取后进入异常清理，`sudo security remove-trusted-cert -d` 等待 120 秒超时。

本机只读对照已安装的已签名 App：`codesign -d --extract-certificates <prefix> <app>` 将 prefix 误当成输入文件，返回“文件不存在”；使用 `--extract-certificates=<prefix>` 则正确导出 leaf DER。修正等号参数，移除一次性 VM 上的交互式信任撤销，并保留限时及私钥清理。

运行 [42375776772](https://github.com/CodeKillerCoser/codepet/actions/runs/42375776772) 已证明两个不同测试程序的 DR 都是 `identifier "com.codepet.signing-probe" and certificate root = H"5fa53f2c971be12909e9b2c9cfcc86e63632a590"`。此前只接受 `anchor` 的检查不符合自签名 DR 输出，现同时接受精确证书约束。完整 App 和 DMG 签名完成后，日志显示 Tauri `Cleaning …/Code Pet.app`，导致后续检查已删除路径而失败；[Tauri 打包源码](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-bundler/src/bundle.rs) 也明确仅请求 DMG 时清理中间 App。现请求 `app,dmg` 保留校验对象。失败时保留已生成 DMG 供诊断，但发布 job 仍要求构建成功；失败运行的产物不可视为已验收包。

签名脚本 11 项测试通过，覆盖实际 root DR 格式和公有验签诊断。运行 [42376418620](https://github.com/CodeKillerCoser/codepet/actions/runs/42376418620) 的首次尝试中，Rust 编译实际耗时 11 分钟；13:34:36 UTC 完成两个 bundle、App 深度验签、固定证书提取和 DR 校验，DR 为 `identifier "com.codepet.desktop" and certificate root = H"5fa53f2c971be12909e9b2c9cfcc86e63632a590"`。此前因运行耗时作出的取消请求恰好阻止了上传，已单独重跑 macOS job；不能把耗时偏长直接认定为签名等待。

同一运行的第二次尝试已成功，构建提交 `3c91fd4d6238488c89ef7d5c8fb6b2768c510181`，[macOS job 128418395334](https://github.com/CodeKillerCoser/codepet/actions/runs/42376418620/job/128418395334) 在 13:46:41 UTC 通过最终 App 验签并清理 keychain，随后上传 [macos-arm64-dmg](https://github.com/CodeKillerCoser/codepet/actions/runs/42376418620/artifacts/13576188873)。运行参数 `publish=false`，只生成 Actions artifact，未发布 GitHub Release。

下载到本机 `~/Downloads/codepet-signed-3c91fd4/Code Pet_0.3.9-beta+3c91fd4_aarch64.dmg`，SHA-256 为 `8d7617e1c4b5893cc91f84be4d96fc8be509ac5776c071c2040c4f8c7225f8eb`。`hdiutil verify`、DMG 的 `codesign --verify --strict` 均通过；以只读方式挂载镜像，对其中 App 执行 `codesign --verify --deep --strict`、提取证书并核对指纹，均通过。App 版本为 `0.3.9-beta+3c91fd4`，Authority 为 `Code Pet Development Code Signing`，DR 与 CI 一致；检查后卸载镜像，未安装或启动新包。

参考：[Apple 证书与签名身份](https://developer.apple.com/documentation/technotes/tn3161-inside-code-signing-certificates)、[Apple 自签名证书用途](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/Procedures/Procedures.html)、[Tauri 签名配置](https://v2.tauri.app/distribute/sign/macos/)。
