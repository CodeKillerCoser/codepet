# macOS 固定自签名开发证书与 CI 签名

## 背景与目标

Code Pet 默认 `signingIdentity: "-"` 为 ad-hoc 签名，代码变化可能改变系统用于识别应用的 designated requirement。开发分发需要复用同一张非 Apple 代码签名证书，保持证书身份与 `com.codepet.desktop` 标识一致。证据见 [文件授权排查](macos-file-access-authorization.md)。

本方案不提供 Apple 公证或 Gatekeeper 信任，不自动授予文件权限。首次从 ad-hoc 切换证书身份可能需要重新授权；后续更新的 TCC 保留必须实测。

## 实现与涉及模块

- `scripts/macos_signing.py generate`：一次性生成 RSA 3072 / SHA-256、有效期 3650 天的自签名代码签名证书，导出加密 PKCS#12。目录权限 0700，文件 0600，拒绝写入仓库或覆盖已有目录，防止误提交私钥或每次构建换身份。
- `.github/workflows/release.yml` 的 macOS job：通过三个 Secret 向 `macos_signing.py build` 提供固定身份。Windows updater 签名和本地默认 ad-hoc 配置保持原流程。
- `build`：只接受 GitHub 托管 macOS runner；导入前校验公有证书指纹和至少 30 天剩余有效期。临时 keychain 只为 codesign 开放私钥；自签名证书在临时 runner 上仅为 codeSign 策略设置信任，随后用指纹覆盖 `APPLE_SIGNING_IDENTITY`。Tauri 子进程不接收证书 Secret，也不沿用其他 Apple 账号/证书环境变量。
- 验收：先签两个不同测试二进制，比对证书和 designated requirement 一致且不依赖 cdhash；再构建 App 并验证深度签名、证书指纹和应用标识。成功或异常退出均尝试恢复 keychain 列表、移除临时信任、删除 keychain。硬终止由 GitHub 销毁一次性 runner 兜底。
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
- 信任设置遗留：仅一次性托管 runner 执行导入；异常测试检查全部清理动作。实际 CI 检查 cleanup 成功，不支持用本机伪造 runner 环境运行。
- 自签名不被构建工具接受：CI 的双二进制 smoke 在完整构建前失败；上线验收需确认 smoke、App 验签和 DMG 产物上传均成功。
- TCC 授权仍重复：用同一证书的两个不同版本，在测试 Mac 上授权版本 A 的 Documents/Downloads，替换版本 B、退出重启，再经 Code Pet 自身访问相同目录。记录弹窗应用名、版本、DR；当前终端能访问不构成验证。

## 本轮验证与未知项

`python3.12 scripts/macos_signing_test.py`、`python3.12 scripts/package_signed_test.py`、`node --test scripts/generate_latest_json.test.mjs`、`actionlint .github/workflows/release.yml`、`git diff --check`。生成了真实证书并解析验证加密 p12，未修改本机 keychain/信任设置。完整 CI 签名、公有 DMG 安装行为和 TCC 跨版本保留尚未验证。

参考：[Apple 证书与签名身份](https://developer.apple.com/documentation/technotes/tn3161-inside-code-signing-certificates)、[Apple 自签名证书用途](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/Procedures/Procedures.html)、[Tauri 签名配置](https://v2.tauri.app/distribute/sign/macos/)。
