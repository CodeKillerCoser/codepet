# Agent 事件已知风险

## 后台 Agent 噪声

Codex 可能产生记忆总结、个性化建议、任务标题生成等内部后台任务。部分已知模式在 `frontend/lib/activity.ts` 中硬编码过滤，用户自定义过滤保存在设置里。

风险：只依赖硬编码过滤，无法覆盖未来新增的内部 prompt。

验证：修改过滤逻辑时，为标题和消息过滤补前端测试。

## 孤立终态事件

没有稳定 identity 的终态 `done` 事件可能生成误导卡片。当前归并逻辑会丢弃孤立 completed session，除非它能匹配到可见 active provider card。

验证：保留 `frontend/lib/activity.test.ts` 中覆盖孤立 completed session 的测试。

## Provider 字段漂移

Agent 可能改变 payload 字段。归一化逻辑应兼容别名，并在内部保留 raw data，同时向前端发送清理后的事件。

验证：修改提取规则前先新增 event normalizer fixture。
