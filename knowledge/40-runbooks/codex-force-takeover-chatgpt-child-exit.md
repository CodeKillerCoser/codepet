# 强制接管与 ChatGPT 子进程异常

## 现象与证据

2026-11-10 用户反馈手机接管后 ChatGPT 经常提示运行错误，要求核对是否仅关闭了
App Server、是否重启桌面 App。本次仅做日志与源码排查，没有关闭真实进程。

- 手机导出包 `codepet-logs-2026-11-10T14-46-44-147255Z/codepet.4.log:1566–1571`：
  北京时间 15:11:08，resume 约 280 ms 收到 `conversation_write_conflict`。之后三次
  acquire 快速失败；包内没有 RPC timeout，也没有后续强制 resume 的记录。LAN 候选
  建连 timeout 与已连通 RTC 上的接管 RPC 是不同链路。
- 手机目标 `device-8819250a…`，本机 identity 为 `device-8d247c27…`，不是同一设备。
- 本机 `Library/Logs/com.openai.codex/2026/11/10/` 下进程 65049 的桌面日志，4195 行：
  `2026-11-10T04:09:05.093Z`，`Codex CLI process exited classifiedAsExpected=false`
  且 `signal=SIGKILL`；4196 行立即 `fatal_error_broadcasted`，4216 行又报告
  `Hosted AppServerManager identity is not ready`。这证明子进程异常退出会触发用户看到的
  运行错误，但没有信号发送者证据，不能断言该次退出由 CodePet 造成。
- 当前本机进程树确认 ChatGPT 主程序会启动
  `ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex`。

## 源码行为与缺口

`provider.rs::conversation_acquire_interaction` 普通接管调用原生 thread/resume，必要时
恢复 CodePet 自己的 runtime；遇到外部 writer 直接返回冲突，不退出外部 App。

显式 force 先校验活动、确认 writer 冲突，才调用 `desktop_takeover.rs::close_desktop`。
它保护有 harness 标记的进程、CodePet 子树与祖先，结束其余候选。macOS 的 `kill()`
经当前 sysinfo 实现发送 `Signal::Kill`，不是应用级正常退出；代码没有重新启动 App 的步骤。

修复前 `desktop_root` 识别 `/Codex.app/Contents/MacOS/Codex`，却不识别
`/ChatGPT.app/Contents/MacOS/ChatGPT`；`codex_process` 按可执行文件末尾 codex 匹配
上述 ChatGPT 内部子进程。因此当保护条件不适用时，会留下 ChatGPT 主程序而终止其运行时。
它也不限于 app-server 参数，其他未标记 codex 进程可能被选中。该规划缺口与 SIGKILL
故障机制吻合，但原始现场关联仍未证实。引入该缺口的提交未确认。

“立即释放”是另一条路径：停止并重启 CodePet 自己的共享 Server，清空旧交互，保持手机
暂停状态；不是重启 ChatGPT/Codex App。

## 按用户要求修复与验证

用户随后明确要求先杀 Codex / ChatGPT App，再杀其他 Codex CLI 或 App Server。
已补齐 macOS ChatGPT 精确 bundle 主程序路径，复用既有桌面优先排序；普通 acquire
仍不关闭外部进程。保留强制终止行为，不增加自动重启，也不按任意 ChatGPT 文件名扩大
候选范围。Windows 既有识别不变。Remote 的强制接管提示同步说明桌面 App 的影响。

进程规划测试覆盖两个 App 都先于较小 PID 的 CLI、结束 ChatGPT 失败后不再处理 CLI、
CodePet 自有 harness 同路径保护、祖先/标记保护及非主程序路径排除。9 项单元通过。
Provider `provider_vertical force_takeover` 2 项集成测试通过，Remote 会话页面 `force takeover`
6 项测试通过，`git diff --check` 通过。
这些测试验证的是终止调用顺序，不是 OS 同步退出保证；既有末尾等待确认进程消失。
真实 App 关闭、自动拉起和写锁转移仍需在隔离桌面实例验收，未在用户运行任务的 App 上强杀。

需要补齐原始卡住时间和目标设备日志。此前 45/75 秒期限调整仅解决可复现的等待预算问题，
不是手机日志中 writer 冲突的修复。新代码尚未发布或替换安装版。
