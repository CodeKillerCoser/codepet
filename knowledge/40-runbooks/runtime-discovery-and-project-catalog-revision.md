# Mac 登录环境漏发现 CLI 与 Harness 过早 Ready

## 现象

2026-09-07，Mac mini chensi-3 的 Code Pet 0.1.5+e3a5844 显示 `runtime-not-found: Claude Provider did not find a compatible local runtime`；Android Remote 连接后 Codex 项目列表为空。两项问题需要分别验证。

## 证据

- 用户提供 `Desktop/logs/logs(1)/code-pet.log`：13:40:21.849 Claude 插件初始化成功。初始化成功只证明插件可用，不证明已经找到 Claude CLI。
- Remote 导出日志：09:58:59Z 有成功的 `project.list`，但 10:46:24Z 连接同一设备时报告 0 个项目，随后没有项目请求。不能把连接快照的零计数解释为原生项目不存在。
- SSH 实测 chensi-3：`SHELL=/bin/zsh`；普通命令环境和 `/bin/zsh -lc 'command -v claude'` 找不到文件，`zsh -lic` 返回同名函数。`~/.zshrc` 才追加 `$HOME/.npm-global/bin`，`~/.npmrc` 为 `prefix=~/.npm-global`。实际 `~/.npm-global/bin/claude --version` 返回 `2.1.142 (Claude Code)`。
- Mac mini 的 `/Applications/ChatGPT.app/Contents/Resources/codex app-server` 独立只读探针完成 initialize，`project/list(limit=100)` 返回 12 个项目，无下一页；没有发送模型请求。
- Codex Provider 回归测试在修复前稳定失败：首次 Ready 临时目录与后台发现 ProjectList 后的完整目录 revision 完全相同。
- 原始 APK 来自用户提供的 Actions 34055757635 压缩包，包名 `com.codepet.remote`、versionCode 2、versionName 0.1.1。已安装并启动独立 AVD `codepet_repro_34055757635`（emulator-5556），用户已连接 Mac mini 并确认复现。APK 非 debuggable，`run-as` 无法读取私有日志；这不代表没有日志。

## 根因

Claude：SDK 只运行非交互登录 shell `-lc`，缺少 `.zshrc` 中的 npm PATH。此外 `command -v` 可能返回函数名，直接把整个 stdout 当路径也会受到启动提示影响。此例是发现路径遗漏，不能归因为版本不兼容。

Codex：Harness 在探测前就发布 Ready，Remote 因此可能读取并缓存不完整能力；后台更新沿用同 revision，旧缓存又阻止项目补拉。用户确定的修复边界是推迟 Ready，而不是依赖探测过程不断修改 revision：所有项并发完成后一次发布完整快照，Remote 在 Ready 前不加载数据。

## 引入历史

Codex 后台探测路径由 `1c4e33d` 引入。Claude 的非交互 shell 限制在此次 SDK 发现逻辑中确认，首次引入提交未确认。

## 修复方案与涉及模块

- `sdk/rust/codepet-provider-sdk/src/local_runtime.rs`：读取交互登录 shell PATH，以 NUL 分隔避免普通启动提示污染，再查找真实文件。使用 `$SHELL`，缺失时 macOS 回退 `/bin/zsh`，其他 Unix 回退 `/bin/sh`；不写死 npm 安装前缀、不执行同名函数。沿用 SDK process、5 秒预算、后台发现和 canonical 去重；Windows 分支不变。
- 三个 Provider 的 `src/provider.rs`：启动完成通信握手后返回 Starting；各项探测并发执行，整轮结束后在生命周期锁内一次应用版本/能力/账号/用量并发布 Ready。重复 start 返回当前状态，stop/shutdown 取消探测并使旧 epoch 失效。启动 RPC 无需等待慢探测，不扩大 Host 超时。
- Codex 保留原有 revision；临时增加的 `:catalog:<epoch>` 已撤回。Host 现有原子合并逻辑可保留早到的 Ready，不会被迟到 Starting 回包覆盖。
- `codepet-remote/lib/application/sessions/device_session.dart`：Starting 时跳过 describe 和数据列表；Starting→Ready 后不用临时能力缓存，合并同代同 revision 的在途 describe，完整能力返回后一次补拉项目和会话。重复心跳不重复加载。
- Provider 与 Remote 测试更新为等待 Ready 通知/快照；Claude fixture 按 canonical 父目录比较数据路径，避免 macOS `/var` 与 `/private/var` 等价路径导致伪失败。

## 验证结果

- SDK local_runtime：7 项通过，含交互 PATH、同名函数、启动提示、空格中文目录；修复前新增测试失败。
- Mac mini 独立 Host native_runtime 探针：修复后的 Claude Provider 未指定 CLI 路径，成功发现 1 个安装，约 2.03 秒通知库存，refresh 重扫通过。首次运行出现过 mux 握手超时，复测成功；首次超时原因未确认。两份临时探针二进制 codesign 验证通过。
- 三个 Provider `provider_vertical`：Claude 15 项、Codex 48 项、OpenCode 3 项通过，2 项真实 smoke 默认忽略；覆盖 Starting、并发探测、一次完整 Ready、停止取消与旧结果隔离。
- Codex 全量默认高并发运行中，饱和 EOF 测试触发过 2 秒退出预算失败；该项单独运行通过，整套以 `--test-threads=4` 运行通过。没有通过放宽退出预算掩盖失败。
- Host `manager_gateway`：24 项通过。
- Remote `flutter test test/devices/device_session_test.dart`：41 项通过，新增 Starting 无请求、同 revision Ready 后只读取一次、重复状态和在途请求合并验证。
- 本机三个 Provider 的 `native_runtime -- --ignored --nocapture --test-threads=1` 均通过：真实 CLI 发现、选择、隔离数据目录启动，先返回 Starting，再等待完整 Ready，无模型请求。
- Remote 首页、会话详情和 device_session 三组测试共 106 项通过；涵盖布局、动作可用性与异步数据生命周期。
- `git diff --check` 通过。未运行格式化工具，保留用户原有架构文档与 hook 清理文档改动。

## 回归防线与风险

- [Provider 后台信息规约](../60-rules/provider-background-details.md)：Harness 完整 Ready 是数据加载门槛；不缓存 Starting 的临时目录。
- [本地路径规约](../60-rules/provider-local-runtime-paths.md)：shell 配置与 GUI PATH 不同，检查真实候选路径，不从错误文案推断未安装。
- shell 交互启动可能有慢脚本：仍在后台并保留 5 秒预算与进程树清理；需用慢 shell/实机库存检查排除阻塞回归。
- SDK 修改同时影响 Claude/Codex/OpenCode 的 Unix shell 发现；Windows 保留独立实现，通过各 Provider 与 SDK 检查验证。

## 未知项

修复的新启动边界的安装版端到端验收结果另行补充；当前 APK 已复现不等于已验证修复后的 Host。原生返回项目数证明数据存在，不证明每个项目的会话详情均已验证。
