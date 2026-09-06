# Provider 透传分页与 item 文本截断

## 规则

Provider 不为超限调整 caller 的 cursor/limit，不设置整 turn、整页或 item 总字节预算。仅在生成 `kind: tool` 的 item 时检查超大文本；截断当场记录到该 item 的可选 `_meta`。所有七种 item variant 都允许开放的 `_meta`，未提供时省略。

## 适用场景

Codex 历史读取、三个内置 Provider 的 item mapper、Agent schema 与生成 SDK、Host/Gateway 透传、Remote 历史和工具展示。内容策略属于 mapper；SDK 提供可选 helper，stdio Runtime 不自动应用它。

## 反例

- caller 请求 20 turns，Provider 内再拆成 10 turns、根据版本切成逐 item 读取，或短页自动补齐。
- 按累计 turn/page 大小删除 item、返回占位历史，或因为原生 JSONL 超长关闭共享 Server。
- 截断后只放一个布尔值，或把记录散落到 input、content block 和 item 三处。
- 因 structured JSON 大就转换成 opaque，或对 command/message 等独立 item 同时套用 tool 策略。

## 推荐做法

Codex get 先 `thread/read(includeTurns=false)`，再一次 `thread/turns/list(itemsView=full, sortDirection=desc)`，原样传递 caller cursor/limit（省略时默认 20），返回原生 nextCursor。协议仍验证 limit 在 1–100；这不改变合法请求数量。当前页恢复为时间正序，客户端仅加载首屏，后续页由用户上翻手动请求，按稳定消息身份去重后前插，不覆盖实时内容。参见[消息数据源与 LRU 规约](remote-conversation-message-sources.md)。删除版本判断、item probe/hydration/fallback、Provider 内 10-turn 分批与短页补齐。Harness 版本只用于描述。resume 继续 `excludeTurns=true`，Gateway 复用 acquire/get 返回首屏。

原生 JSONL 流式解码，不以整行大小拒绝响应；坏行 drain 到换行，只影响对应请求，真正 EOF/I/O 故障仍清理 Server。完整原生对象仍会占用内存；这次不引入另一种隐式 turn 预算。

`sdk/rust/codepet-provider-sdk/src/item_text.rs` 的 `truncate_tool_item_text` 只匹配 `ToolConversationItem`。默认每个文本字段 256 KiB，按 UTF-8 字节安全保留首尾，中间放 `\n…\n`；`retainedBytes` 包含该标记。检测 tool input 的 command/opaque 文本、structured JSON 字符串叶子，以及 outcome 的文本块和结构化字符串叶子。对象、数组、数字、布尔值、稳定 ID、媒体和资源 URI 保留；独立 command、file-change、message、reasoning、approval、unknown item 暂不检测。不设置多个文本字段的累计预算。

记录形状如下，`path` 是相对 item 的 JSON Pointer（键名按 `~0`、`~1` 转义）：

```json
{
  "_meta": {
    "truncations": [{
      "path": "/tool/outcome/content/0/text",
      "originalBytes": 20971520,
      "retainedBytes": 262144,
      "strategy": "head-tail"
    }]
  }
}
```

`truncations` 是保留键，已有合法记录可追加，其他 `_meta` 键原样保留。没有截断不新增 `_meta`。历史与 item upsert 复用 mapper；截断不推迟到 IPC 编码。旧 input/content block 的 `truncation` 字段保留读取兼容，新策略只写 item `_meta`。

Host/Gateway 透传 Agent item。Remote 的 `GatewayMessage.meta` 保留原对象，mapper 按 input 或 outcome content 路径汇总已有截断提示，不复制正文。旧 SDK 严格拒绝未知字段，因此 Host/Provider 与 Remote SDK 需配套更新。

Provider Frame V1 最终 header + raw/zstd payload 的 16 MiB 上限保持不变。该层超限仍按原 id 返回 `provider_response_too_large`；Remote 保持 cursor，按 20→10→5→1 调整自己的请求，成功后才推进。巨大非 tool 正文、媒体或很多中等文本仍可能触发此错误；不得因此在 Provider 偷改分页或 turn 大小。事件编码拒绝不关闭 Server，但不承诺自动重放被拒事件。

## 来源

- 2026-09-06 用户明确约束：Provider 不改数量、不限 turn，仅 tool 文本检测，截断信息当场放可选 item `_meta`。
- [原生历史尺寸与版本排查](../40-runbooks/codex-detail-size-and-capability-loading.md)：旧 full-turn 响应 24,756,517 字节引出原生行上限问题；本规则替代当时的版本驱动 item 分页方案。
- [共享 Server 错误隔离](../40-runbooks/codex-request-errors-must-not-stop-server.md)与[载荷唯一所有权](conversation-tool-payload-ownership.md)。

## 验证方式

- Agent schema 测试和 Rust round-trip 覆盖全部七种可选开放 `_meta`；Rust wire 必须为 `_meta`，不能误写成 `meta`。生成器 freshness 与独立导出 cargo test 防止 SDK 漂移。
- SDK helper 验证 UTF-8、首尾保留、结构/ID、JSON Pointer、原有 metadata、无截断时省略与重复应用不重复记录。
- Codex 进程测试 `provider_binary_forwards_turn_limit_and_truncates_only_tool_text_with_item_metadata`：原生 20 MiB tool 文本正常解码，仅 tool 被截断；limit=20、短页 nextCursor 原样返回；普通消息与 command 输出保持完整，同 PID 继续服务。0.151/0.152/Host userAgent fixture 都用 full turns，不调用 item API。
- 真实 Codex 0.153.1 只读探针在正常 Host 环境完整读取两条历史，各 2 页，669 与 1,614 items，随后同 Server 项目查询成功；不发送 turn/resume。诊断只保留计数和状态。
- `cargo test --manifest-path crates/Cargo.toml`、Provider SDK 测试、`npm run protocol:check`、SDK 导出测试及 Remote 全量 `flutter test` 验证跨层兼容。Remote gateway 测试确认 metadata 与既有截断提示；保留历史合并、delta、composer 与布局回归。
- 内存风险通过真实大历史探针持续观察；不把“无原生字节上限”误写成“无内存成本”。不支持 full turns 的旧 CLI 仍返回真实错误，未声称所有版本均实测。

2026-09-06 验证结果：crates workspace 200 项通过、2 项原有外部依赖测试忽略；Provider SDK 28 项、协议 17 项、SDK 导出 3 项、导出包 Rust 4 项、Remote 261 项通过。Remote 定向 analyze、Tauri `cargo check --lib` 和两仓库 `git diff --check` 通过。Tauri `cargo check --tests` 被现有 `tests/macos_window_tests.rs` 引用旧 `src/macos_window.rs` 阻塞，实际文件已在 `src/platform/`，此无关路径本次未改。

Host app/DMG 与 Android release 构建成功，app 签名、DMG 校验、APK 签名通过；最终包内 Provider 重复完成上述两条真实历史与后续项目查询。配套产物为 Downloads 的 `CodePet-release-20260905-233402` 目录下 `CodePet-Host-0.1.4-macOS-arm64-item-meta.dmg` 和 `CodePet-Remote-0.1.0-Android-release-item-meta.apk`；build-info、SHA256SUMS 与 build-logs 保存验证信息，尚未覆盖运行中的安装。
