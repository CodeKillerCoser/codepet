# 用户消息文件引用与正文分离

## 规则

用户消息通过现有 Agent/Gateway `MessageConversationItem.contents` 保留文本、图片和资源引用。Provider 不得只提取文本而丢弃图片输入；Remote 的用户消息展示模型必须保留附件块，不能只保留合并文本。文件名是展示元数据，不是访问授权。

## 适用场景

Codex Desktop 发起的带图片或文件引用的消息，经 Codex Provider 映射、Gateway 会话查询进入 Remote 气泡。

## 来源与证据

2026-09-09 用户截图显示文件清单、电脑临时路径及 `My request` 模板全部成为气泡正文。本机 CodePet 日志中 sequence 6059 为 `UserPromptSubmit` hook，6060 为其 Provider observation 包装；两者都含完整文件清单 prompt，**不是 Gateway 发给 Remote 的业务消息抓包**。

对应原始会话记录包含文件清单 `input_text`、图片路径标记、独立 `input_image`（data URI）及结束标记。源码中 Codex `UserMessage.text_inputs: Vec<Option<String>>` 将非文本输入置为 None，mapper 再过滤；Remote `UserMessageBlock` 也只有 text。这两处共同导致附件结构丢失/不可见。

## 推荐做法

- Provider 的 `CodexUserInput` 分别保存 text、image、localImage；使用既有 text/image/resource-link 内容块投影，不新建重复的 wire 附件协议。
- 只识别完整、匹配的 Desktop 文件清单模板（标题、文件名与绝对路径、固定说明、请求分隔符）；匹配成功后输出文件资源块与请求正文。普通 Markdown、部分模板、文件名和路径不一致的文本保持原样。
- 本地引用转换为转义后的 file URI；路径按源平台语义处理，不在解析时读取文件。原生 localImage 与清单同指一个文件时，保留命名资源的稳定 contentId 并补充图片类型，避免重复标签。
- Remote 在用户消息模型中保留 attachments，正文和标签分开渲染。标签使用 name，缺失时使用 URI 文件名或媒体类型兜底。电脑 file URI 不可交给手机本地文件打开器；预览/下载需要独立的 Host 资源访问链路。

## 验证方式

`cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --lib`：63 项通过，覆盖原生图片、文件清单拆分、路径转义和稳定内容 ID。

Remote 的 `user_message_attachments_test.dart`、`conversation_links_test.dart`、`conversation_timeline_test.dart`：13 项通过，覆盖 Gateway 解码到消息投影、360 像素窄屏长文件名、纯附件消息，以及原有链接行为。当前结果为源码测试，仍需新版 Provider 与 Remote APK 的真机联合验收。
