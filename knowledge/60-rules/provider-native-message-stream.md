# Provider 原生消息流与身份

Provider 必须把原生“进入消息历史”的用户输入事件转换为标准消息 upsert。普通发送、Host 队列和 steer 共用此消息流；RPC userItem 回执不能替代流式消息，Host 不根据队列文本自行合成消息。原生接纳事件不能冒充历史提交。

正文、推理及工具的标识必须包含原生定义的类型/消息作用域。OpenCode 2.x 的 text 与 reasoning ordinal 各自从零开始，不能使用数组全局下标。历史与流式的身份转换必须一致。

验证新适配版本必须含推理与正文共存、连续轮次、用户输入和 steer、取消未送出的输入、同一消息的响应/流/历史去重。对未验证版本和上游已知限制明确记录，不能从 schema 存在推断行为可用。

证据与验证命令见 [OpenCode 消息流修复](../40-runbooks/opencode-input-stream-and-content-identity.md)。

错误展示只能提取原生错误的可读 message（兼容 error/data 内嵌消息和纯文本），不得把 response body、重试元数据或整个对象序列化到会话正文。历史失败、实时失败摘要和工具错误共用同一提取规则；没有可读消息时给出明确的通用错误提示。回归需覆盖带嵌套 HTTP body 的 429、纯文本、内嵌 message、空 message 和缺少 message 的对象。

合并终态错误链路时保留 ProtocolError 的 code、retryable 和安全元数据；可读消息提取通过 execution_error 接入历史与实时失败，继续兼容 OpenCode 1.x 的 HTTP 错误包装解析。
