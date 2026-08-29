# Codex Desktop IPC 必须 Fail Closed

## 规则

Codex Desktop socket、握手、路由、协议版本或 revision 无法确认正确时，Provider 必须明确 unavailable/error，并停止发布不可信状态。不得启动独立 App Server、恢复 Hook、扫描 transcript/audit 或监听文件作为回退。

Desktop 私有 DTO、方法名、路由字段和原始 JSON 只能存在于 Codex Desktop adapter。未完成端到端验证的审批、回复、快捷回复、steer、interrupt 等动作不得声明 capability。

## 适用场景

- 修改 Codex Desktop IPC transport、Owner/Follower adapter、状态 mapper 或重连逻辑。
- 修改 Runtime Gateway Provider 状态、capability 或 ProviderExtension。
- 修改桌宠任务状态、审批按钮、回复入口或 Provider unavailable 提示。
- 升级 Codex Desktop，或调整支持版本范围。

## 反例

- socket 连接失败后静默执行 `codex app-server --listen stdio://`，让 UI 看起来仍在线。
- patch revision 跳跃时继续合并，生成一个从未存在过的任务状态。
- 为了发现晚加入任务而扫描 transcript，并把推断结果标成 Desktop 实时状态。
- 把私有 follower payload 放进 Standard Protocol extension，让前端按原生字段分支。
- 私有协议存在审批请求，就直接广告 `approval.resolve`，但没有验证目标路由和 Desktop 是否确认处理。

## 推荐做法

- 连接前检查解析后 socket 的文件类型、有效用户 owner 和 `0600` 权限；失败时提供安全、可诊断原因。
- 每个物理连接使用唯一 client identity，严格过滤定向消息，并单独处理广播。
- snapshot 建立 revision 基线；只应用连续 patch。缺口进入等待 snapshot，重连清空全部连接态。
- mapper 只挑选已验证的公共状态；未知私有事件安全忽略并记录有限诊断。
- capability 使用肯定列表，只包含当前 adapter 已实现并测试的方法。阶段一保持只读，无审批和回复。
- 无任务目录时明确报告覆盖限制，不用推断数据伪造目录。

## 来源

- `../50-decisions/codex-desktop-ipc-as-provider-source.md`
- `../30-domains/agent-control/codex-app-server.md`
- 当前 Codex Desktop 私有 Owner/Follower 实现与只读多客户端探针。

## 验证方式

- frame codec 覆盖 little-endian、短帧、非法 JSON 和 256 MiB 上限。
- 多 client fixture 覆盖定向隔离与广播分发。
- revision 测试覆盖顺序、重复、缺口、等待 snapshot 和重连复位。
- Provider 测试断言阶段一不广告审批、回复、快捷回复和其他未接通写能力。
- 前端测试断言 capability 不可用时不渲染相应按钮。
- 静态检查确认独立 App Server 启动命令已从生产路径删除，Desktop 私有方法名没有越过 adapter 边界。
