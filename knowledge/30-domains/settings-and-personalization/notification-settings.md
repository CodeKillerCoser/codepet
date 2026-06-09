# 通知设置

## 当前设置

`NotificationSettings` 包含：

- 选中的声音。
- 自定义声音路径。
- 权限、失败和完成状态的响铃开关。
- 等待审批的重复秒数。
- 静音时段。
- 外部机器人通知：全局启用开关、等待授权/任务失败/任务完成触发项、消息模板，以及钉钉或飞书渠道列表。

抽打反应音存放在 `PetSettings` 下，因为它属于宠物互动，而不是任务通知。

## 运行时行为

`frontend/PetApp.svelte` 在新事件到达时调用 `handleRing()`。它使用 `frontend/lib/sound.ts` 判断是否响铃、静音时段、声音播放和重复行为。

外部机器人通知由后端处理，避免依赖桌宠窗口是否打开。`src-tauri/src/app/notifications.rs` 负责事件触发判断、消息模板渲染和 provider 策略分发；`collector`、Codex audit watcher 和 Claude transcript watcher 在推送实时事件后调用同一通知入口。终态通知会按 provider、事件类型和 session 生成短期幂等 key，避免 hook 入口和 transcript/audit watcher 同时报告完成时重复推送。

钉钉支持两种渠道：

- 企业内部机器人：使用 `robotCode`、`clientId`、`clientSecret` 获取 accessToken，再按 userId 或 openConversationId 发送。
- 群 webhook：使用 webhook URL，可选加签密钥。

飞书支持群 webhook，可选签名密钥。

机器人通知先把事件抽象为变量，再渲染模板。`RobotNotificationSettings.template` 包含 `title`、`header`、`primary`、`secondary`、`footer` 五段，默认变量包括：

- `{{statusIcon}}`、`{{status}}`、`{{agent}}`、`{{task}}`、`{{time}}`。
- `{{content}}`、`{{contentBlock}}`。
- `{{cwd}}`、`{{cwdLine}}`。
- `{{tool}}`、`{{toolLine}}`。
- `{{sessionId}}`、`{{sessionLine}}`。

变量值会按 Markdown 场景做基础转义；空变量所在的模板空行会被压缩，避免卡片留下孤立空白。provider 策略只消费渲染后的五段内容，不应直接拼事件字段。

机器人通知使用结构化 Markdown 样式：

- 钉钉群 webhook 使用 `msgtype=markdown`，正文写入 `markdown.title` 和 `markdown.text`。
- 钉钉企业内部机器人使用 `msgKey=sampleMarkdown`，`msgParam` 序列化为 `{"title": "...", "text": "..."}`。
- 飞书群 webhook 使用 `msg_type=interactive`，通过卡片 header 显示标题、摘要和状态色，在 body 中使用 `markdown` 元素区分一级内容、二级内容和 footer。

相关公开文档：

- 钉钉开放平台：自定义机器人和企业机器人 Markdown 消息类型。
- 飞书开放平台：自定义机器人支持 `interactive` 卡片，卡片 body 可使用 `markdown` 元素。

机器人敏感字段保存在本地 settings 中，不应写进源码默认值、测试夹具或日志。通知错误日志需要避免输出 token、sign、clientSecret 等敏感值。

## 验证

- 运行 `npx vitest run frontend/lib/sound.test.ts`。
- 运行 `npx vitest run frontend/lib/api.test.ts frontend/styles.test.ts`。
- 运行 `npm run build`。
- 运行 `cargo test --manifest-path src-tauri/Cargo.toml --test settings_tests`。
- 运行 `cargo test --manifest-path src-tauri/Cargo.toml app::notifications::tests`，覆盖钉钉 webhook、飞书 webhook、钉钉企业机器人 token+发送的本地 mock 链路。
- 人工确认审批被处理或 activity 消失后，重复审批提醒会停止。
- 人工在个性化页添加机器人渠道，使用测试按钮确认渠道配置可投递。
