# Transcript Path Display

## 规则

Agent transcript/session JSONL 路径只能作为元数据或排查线索，不能作为桌宠任务卡片的可见标题或正文。

## 适用场景

修改 `frontend/lib/activity.ts`、事件归并、卡片标题/正文、Codex/Claude/Qoder/Cursor hook payload 解析或 Windows 路径处理时适用。

## 反例

只识别 Unix 风格 `/Users/.../.codex/...jsonl`，忽略 Windows 的 `C:\Users\...\.codex\...jsonl` 或 `\\?\E:\.codex\...jsonl`，会导致桌宠把 session 文件路径渲染成对话标题，看起来像乱码。

## 推荐做法

路径判定应同时支持 `/` 和 `\` 分隔符，以及 Windows extended-length 路径前缀。卡片标题遇到 transcript path 时回退到状态或已有 prompt 标题；卡片正文也要过滤 transcript path，不能直接显示。

## 来源

Windows dev 模式下 Codex 工具事件携带 `\\?\E:\.codex\sessions\...\rollout.jsonl`，旧前端只识别 Unix 路径，导致任务卡片显示路径而不是中文对话标题。

## 验证方式

运行 `npx vitest run frontend/lib/activity.test.ts`，覆盖 Windows transcript path 不覆盖 prompt 标题、也不作为 `cardTitle()` 或 `cardMessage()` 输出。
