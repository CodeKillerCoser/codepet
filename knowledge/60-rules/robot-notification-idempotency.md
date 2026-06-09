# 机器人通知幂等

## 规则

等待授权、任务失败、任务完成这类会触发外部机器人的事件必须具备通知幂等能力。同一 provider、同一事件类型、同一 session 在短时间内只能触发一次机器人通知。

## 适用场景

修改 `collector`、Codex audit watcher、Claude transcript watcher、事件归一化或机器人通知发送链路时适用。

## 反例

Hook collector 收到 `Stop` 后发送任务完成通知，同时 transcript watcher 又从同一 session 解析到完成结果并再次发送通知，用户会收到两条完成消息。

## 推荐做法

- 发送机器人通知前生成 dedupe key，优先使用 `session_id`。
- 没有 `session_id` 时再退化到标题、内容、目录和工具等上下文。
- Provider 策略只负责投递，不应各自实现去重。

## 来源

用户反馈任务完成消息会被推送两遍。当前修复在 `src-tauri/src/app/notifications.rs` 的通知入口中增加短期幂等缓存。

## 验证方式

- `cargo test --manifest-path src-tauri/Cargo.toml app::notifications::tests`
- 测试应覆盖同 session 的重复终态事件被抑制。
