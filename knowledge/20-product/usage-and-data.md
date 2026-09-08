# 本地体验、数据与诊断

产品入口见 [README](../../README.md)，远程控制与插件上手见 [远程控制指南](remote-control-and-plugins.md)，构建步骤见 [本地开发、测试与构建](../00-project/development.md)。本页保留本地桌宠设置、数据与问题排查入口。

> 当前本地活动列表已切换为 Provider Hook／原生插件订阅，见 [活动感知参考](../10-architecture/agent-integration-reference.md)。本页音效与旧事件页说明保留既有设置背景，不代表新的只读任务列表已接通全部旧通知或动作。

## 使用方式

1. 启动应用。
2. 在主窗口的 `Agent` 页启用需要接入的工具，例如 Claude Code 或 Qoder。
3. 运行对应 Agent 的任务。
4. 桌宠窗口会展示任务状态、工具调用、授权等待、完成或失败等活动。
5. 在 `个性化` 页调整宠物、主题、任务气泡、通知音效和抽打反应音。
6. 在 `用量` 页查看按 Agent 聚合的 Token 使用情况。
7. 在 `最新事件` 页查看最近收到的 hook 事件。

### 抽鞭子互动

桌宠窗口右侧有抽鞭子按钮。点击后：

1. 播放鞭子抽打动画。
2. 播放鞭子抽打音效。
3. 如果配置了抽打反应音，会在鞭声后继续播放桌宠反应音。

在 `个性化 -> 通知声音 -> 抽打反应` 中可以选择：

- `无`：只播放鞭声。
- `啪`：播放短促拍打反应。
- `啊啊啊`：播放内置叫声。
- `自定义`：选择本机音频文件作为桌宠被抽后的反应音。

自定义反应音和普通通知自定义音是两套独立配置。

### 通知和重复提醒

通知声音支持：

- `blip`
- `chime`
- `bell`
- `custom`
- `silent`

完成、失败、授权等待可以分别控制是否响铃。等待授权属于需要人操作的状态，在用户处理前可以重复提醒；普通任务完成只提示一次。

## 本地数据

应用会在本机写入少量状态和缓存：

- 应用设置入口：系统 local data 目录下的 `code-pet/settings.json`。这个入口保持固定，用于保存自定义数据目录配置。
- 数据目录：可以在 `个性化 -> 系统 -> 数据目录` 修改；未配置时默认仍是系统 local data 目录下的 `code-pet/`。选定的自定义目录必须为空；如果目录已有内容，应用会先弹窗确认，确认后清空该目录再复制原数据目录内容。保存完成后需要重启，日志等运行期资源才会完全切到新目录。
- Token 用量缓存：默认位于数据目录下的 `token-usage.json`。
- 应用日志：默认位于数据目录下的 `logs/code-pet.log`。
- 宠物库：默认位于数据目录下的 `pets`，也可以单独在设置页修改。
- 离线事件暂存：未配置数据目录时仍是 `~/.code-pet/spool/events.jsonl`；配置后使用数据目录下的 `spool/events.jsonl`。

Tauri asset protocol 允许读取 `$APPLOCALDATA`、`$DATA`、`$LOCALDATA`、`$HOME` 和 `$TEMP` 范围内的图片或音频资源，用于宠物图片和自定义音效。

## Token 用量统计

Token 用量统计会读取本机 Agent 审计和 transcript 信息，并生成聚合摘要。当前默认关注：

- `~/.codex/audit/audit.jsonl`
- `~/.qoder/audit/audit.jsonl`
- 审计记录中引用的 transcript 文件

主窗口的 `用量` 页可以选择时间范围和桶大小，查看各 Agent 的用量分布。

## 性能和日志

应用启动时会写入可读的日志 banner，用于区分新日志文件、轮转后的日志文件和每次应用启动。性能事件以 `[perf]` 日志行记录，格式为可解析的 key-value，例如：

```text
[perf] name=startup.total status=ok duration_ms=123 agents=4
```

当前覆盖的主要性能点包括：

- 后端启动总耗时、宠物悬浮窗配置、Agent 配置读取、离线事件回放。
- 历史 Token 用量解析可能读取 audit/transcript；Codex 活动 audit watcher 和启动回放已经移除，不能据此判断桌宠接入状态。
- Token 用量刷新、audit 引用 transcript 数量、递归扫描 transcript 数量和文件体积。
- 主窗口首次刷新、设置读取、Agent 列表、事件快照、宠物库、Token summary、开机自启动状态读取。
- 桌宠窗口设置读取、ready 时间和最近事件同步耗时。

## 排查入口与边界

- 活动没有出现：先检查 Agent 页的来源状态，再参照 [Provider 活动订阅](../10-architecture/provider-hook-observation-proposal.md) 核对 Hook／原生插件安装、原应用信任与首次活动；不再排查旧 Desktop IPC socket 或固定端口 collector。
- 用量来自本机可读记录，不是服务商账单；缺失记录或不兼容格式会影响覆盖范围。
- 启动变慢：参照 [性能监控基线](../40-runbooks/performance-monitoring.md)，用日志区分启动、事件和用量读取耗时。
- 更换数据目录前先备份；核对目标目录内容，并在重启后验证日志、宠物和用量缓存位置。

## 维护依据

设置与交互以 `frontend/App.svelte`、`frontend/lib/sound.ts`、`src-tauri/src/app/settings.rs` 为准；用量来源以 `src-tauri/src/activity/token_usage.rs` 为准。
