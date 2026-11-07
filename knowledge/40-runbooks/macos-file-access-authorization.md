# macOS 文件夹授权弹窗与 Code Pet 权限归属

## 现象与复现路径

2026-11-07，用户反馈通过 Code Pet 使用 Agent 访问 Documents、Downloads 时出现授权弹窗。本轮检查当前安装包、源码和系统设置，未主动触发目录访问或更改授权；未复现同一版本对同一目录重复弹窗。

## 证据

- `sw_vers`：macOS 27.0，26A428。运行进程来自 `/Applications/Code Pet.app/Contents/MacOS/code-pet`；Info.plist 版本为 `0.3.9-beta+4bda6b3`，标识为 `com.codepet.desktop`。
- 系统设置 → 隐私与安全 → 文件与文件夹 → Code Pet：文稿、下载开关均为 on；完全磁盘访问权限页面没有 Code Pet 条目。这是当前 UI 状态，不能据此推断此前每次弹窗的授权结果。
- `codesign -dv --verbose=4` 显示 `Signature=adhoc`、`TeamIdentifier=not set`；`codesign -d -r-` 显示 designated requirement 为 cdhash。源码 `src-tauri/tauri.conf.json` 的 `bundle.macOS.signingIdentity` 为 `-`。
- 安装包 Info.plist 未包含 Documents、Downloads、Desktop 的 usage description；仓库检索未发现专门的 macOS 文件权限引导或状态查询实现。配置中的 assetProtocol scope 只约束 Tauri 资源访问，不授予系统权限。
- 进程树显示 Codex Provider 由 Code Pet 启动，Codex CLI 又由该 Provider 启动。历史现场的 TCC 日志已记录 Git/zsh 访问被归因至 `com.codepet.desktop`，见 [Worktree 目录阻塞排查](worktree-create-directory-access-timeout.md)。本轮未重新采集 TCC 归因日志。
- 用户提出可能复用了固定自签名证书后，进一步读取 GitHub 远端 workflow、两份 Tauri 配置及实际 job 日志。[成功运行 42360671815](https://github.com/CodeKillerCoser/codepet/actions/runs/42360671815/job/128368223518) 构建提交 `4bda6b3`，版本与当前安装包一致；2026-11-07 09:51:34 UTC 明确输出 `Signing with identity "-"`，随后报告缺少公证环境变量并跳过公证。此次 CI 的签名模式已确认是 ad-hoc。
- `.github/workflows/release.yml` 的 macOS job 没有导入 p12、创建签名 keychain 或引用代码签名证书 Secret；`tauri.macos-dmg.conf.json` 只覆盖 DMG/updater 配置，继承主配置的 `signingIdentity: "-"`。`TAURI_SIGNING_PRIVATE_KEY` 仅供 Windows job 的 Tauri updater 签名，不是 macOS 代码签名证书；本轮没有读取任何 Secret 值。

## 初步影响范围

- `crates/codepet-host/src/providers/process.rs::PluginProcess::spawn`：启动 Provider，因此后台 Agent 的目录访问也可能触发 Code Pet 名下的授权。
- `crates/providers/codepet-provider-codex/src/protocol.rs::permission_settings`：FullAccess 映射到 Agent 的 `danger-full-access` / `never`，不代表 macOS 完全磁盘访问授权。
- `crates/providers/codepet-provider-codex/src/provider.rs::create_managed_worktree`：Git 仓库探测仍使用同步 `output()`；授权等待可能表现为上层请求超时，不能直接认定项目损坏。
- macOS 打包配置：临时签名的 cdhash 随构建变化，需要验证替换安装后的授权稳定性。

## 当前判断与已排除方向

访问受保护目录时由 macOS 按需处理文件夹授权，与用户描述相符。Documents 和 Downloads 属于不同授权类别，分别提示并不表示重复授权故障。当前实现缺少应用层提前解释、失败反馈和设置入口。

完整磁盘访问是另一个权限类别，需要用户在系统设置明确授予；仅访问项目所在 Documents/Downloads，不能据此认定必须开启完整磁盘访问。缺少 usage description 也不代表系统不会弹窗，历史日志已有反证。

当前临时签名可能影响升级后的授权识别，但尚无本轮升级前后对照，不能认定它就是用户所述弹窗的原因。

必须区分 ad-hoc 和固定自签名证书。前者的 `-` 是伪身份，不对应证书或固定私钥；后者可以在复用同一证书、私钥、bundle ID 和签名要求时保持 designated requirement 稳定。产物内容、哈希和签名字节仍可随版本变化，“身份稳定”不等于文件签名完全一样。Apple 文档确认自签名证书可以验证不同版本来自同一来源，因此不能把 Developer ID 说成身份稳定的唯一方案；目标系统的 TCC 保留仍需实测，不能把这一能力等同于 Gatekeeper 信任或 Apple 公证。

## 下一步排查与验证

1. 区分首次访问不同目录、同版本重复访问、替换 App 后再次访问；记录弹窗应用名、目录类别和版本。授权后通过 Code Pet 自身发起同一目录操作验证，不能用其他终端的成功代替。
2. 若补充权限体验，先建立统一目录访问接口，再提供 macOS 实现；用户选择项目时说明用途，拒绝后给出设置入口。验证允许、拒绝、取消、重启，以及远程请求等待期间的可见提示与超时恢复。
3. 补充用途文案，并以固定证书身份验证跨版本保留授权；开发用途可验证固定自签名证书，正式分发另考虑 Developer ID/公证。分别检查旧版、新版 `codesign -d -r-`，再执行授权旧版 → 替换新版 → 通过 Code Pet 访问同一目录的对照。用途文案本身不会授予权限。后续已按用户请求生成固定开发证书并配置 GitHub Secrets，工作区流水线改动与待验收项见 [固定开发签名](macos-stable-development-signing.md)。
4. 对 Git 等子进程验证有界等待、取消清理和其他请求不受阻塞，沿用上述 Worktree runbook 的验证路径。

## 已执行与未知项

已执行 `git status --short`、源码检索、`ps -axo pid,ppid,comm`、`sw_vers`、Info.plist 读取、codesign 检查及系统设置只读检查；通过 GitHub connector 读取远端配置与实际 job 日志，`gh run list` 核对最近运行，`git diff --check` 检查文档修改。未修改功能代码或权限，未运行编译测试。尚未验证重复弹窗、签名升级迁移或历史 Worktree 请求在授权后的恢复情况，也未验证用户另行保存的证书是否存在。

平台依据：[Apple 文件访问控制](https://support.apple.com/en-ie/guide/security/secddd1d86a6/web)、[文件与文件夹权限设置](https://support.apple.com/en-au/guide/mac-help/mchld5a35146/mac)、[文稿目录用途说明](https://developer.apple.com/documentation/bundleresources/information-property-list/nsdocumentsfolderusagedescription)。

签名依据：[Tauri Ad-Hoc Signing](https://v2.tauri.app/distribute/sign/macos/#ad-hoc-signing)、[Apple 自签名证书说明](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/Procedures/Procedures.html)、[Apple designated requirement](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/AboutCS/AboutCS.html)。
