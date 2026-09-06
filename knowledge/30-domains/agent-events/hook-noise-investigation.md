# Harness Hook 覆盖与桌宠噪声排查

## 现象

2026-09-06：用户反馈旧 Hook 数据源会投递各种通知，希望重新核对各 harness 的能力，以评估本机多实例活动感知。本文保留原始排查证据；后续实施已按 [Provider 活动订阅设计](../../10-architecture/provider-hook-observation-proposal.md) 收窄为只读桌宠。以下旧链路描述用于解释历史噪声，不代表新链路仍在使用。

## 复现路径

本次只核对官方文档、源码和附近测试，未向实际 harness 安装 Hook，未复现用户历史现场。后续需记录各入口的实际版本，用同一组开始、工具失败后继续、审批、停止、取消、子代理和空闲场景采样。

## 证据

### 官方支持范围

以下是查阅当日文档能力，不代表已安装版本均支持。

- [Codex Hooks](https://learn.chatgpt.com/docs/hooks)：有会话、提示提交、工具前后、权限、压缩、子代理、Stop 和 Interrupt。轮次事件提供 turn_id；子代理使用父 session_id，应保留 agent_id。当前事件表没有 Notification、PostToolUseFailure、StopFailure。PostToolUse 也覆盖非零退出的 Bash。非托管 Hook 需要信任；多来源 Hook 合并运行。旧 notify 配置见[配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)，不能与生命周期事件混为一谈。
- [Claude Code Hooks](https://code.claude.com/docs/en/hooks)：区分 PostToolUseFailure、StopFailure、PermissionRequest、Notification 和 SubagentStop；通知类别包括权限、空闲、认证和 MCP 输入等。Notification 的时机不能替代立即发生的权限事件。
- [Qoder CLI Hooks](https://docs.qoder.com/cli/hooks)：有工具失败、异常停止、权限、通知和 MCP 输入事件；Notification 的 notification_type 可过滤。[Qoder Hooks](https://docs.qoder.com/qoder/hooks) 声明与 CLI 兼容，但设置页显示配置不能证明 Runtime 执行成功。
- [Cursor Hooks](https://cursor.com/docs/hooks)：通用工具事件与 Shell/MCP/File 专用事件并存；conversation_id 跨轮次稳定，generation_id 随用户消息变化。stop.status 区分 completed、aborted、error。主代理、子代理、Tab 和应用生命周期应分别处理。

### 仓库证据

- `src-tauri/src/agent/registry.rs`：Claude 订阅 7 类，Qoder 多订阅 Notification，Cursor 同时订阅通用和专用工具事件；Codex 已禁用。
- `src-tauri/src/agent/hooks.rs::managed_json_entry`：非 Cursor 托管项使用 matcher="*"，Qoder Notification 没有类别白名单。
- `src-tauri/src/activity/events.rs::normalize_hook_payload`：SessionStart 与 UserPromptSubmit 都产生 TaskStarted；PostToolUseFailure 直接产生 TaskFailed 并响铃；Stop、SessionEnd、空闲通知都可产生 TaskCompleted；未知事件产生 Running Message。
- `canonical_hook_event` 把 Cursor sessionStart 合并为 UserPromptSubmit，sessionEnd 合并为 Stop；失败判断未读取 Cursor stop.status。
- `src-tauri/tests/event_normalizer_tests.rs` 中 `post_tool_failure_becomes_failed_status_and_alert`、`qoder_idle_prompt_notification_becomes_task_completion` 明确固化了上述行为。

## 初步影响范围

注册表和安装器决定采集范围；normalizer 决定状态及响铃；collector 和脚本决定转发、离线回放及审批等待；前端 activity 聚合决定重复事件是否成为重复卡片或状态覆盖。Codex 当前仍遵循独立 Companion 数据源，相关旧决策未被本次调研替换。

## 当前假设

代码可确认存在语义混用：工具失败被提升为任务失败、会话建立被提升为任务开始、空闲被提升为完成。它们是噪声候选原因；尚无历史事件样本证明每条路径都对应用户观察。Cursor 重叠订阅、多层配置、子代理和延迟回放也可能增加重复或乱序，需采样验证。

建议采集、聚合、提醒分层：会话事件只维护登记；提示提交建立轮次；工具事件只更新当前活动；权限和输入请求进入等待；主轮次停止区分完成、失败和取消；普通通知默认不展示，未知事件保留诊断而不改变状态。停止 Hook 可能触发继续，不能把收到 Stop 等同于用户目标完成。

## 已排除方向

“各家只有无结构通知”不成立：官方均有结构化生命周期事件。Hook 全局配置也不是历史目录或所有实例的快照，只覆盖加载且实际执行该配置的运行环境。

## 下一步排查

- 按 harness 和版本采样原始事件，保留可用的 session、turn/generation、agent、tool_use 标识；检查子代理归属和迟到事件。
- 验证一次工具失败后继续不产生整轮失败提醒；Stop 后 idle 不重复提醒；取消不显示成功；子代理结束不结束主任务。
- 验证 Cursor 通用/专用工具重叠、多层配置重复、离线回放及多窗口并发；缺少原生轮次标识时明确本地推断限制。
- 恢复 Codex 前分别验证 CLI、Desktop 内置 runtime、IDE 的 Hook 能力与信任流程，再修订双链路决策和隔离规约。
- 本次执行 git status --short、rg 和 sed/cat 阅读，并抓取官方文档；仅新增本文，未运行代码测试，未调用格式化工具。

## 未知项

用户现场版本、原始 payload、噪声频率、变更引入历史及各入口的实际投递覆盖均未确认。Hook 单独提供不了进程崩溃后的可靠终态，也未验证跨进程事件排序与全部审批的可操作性。
