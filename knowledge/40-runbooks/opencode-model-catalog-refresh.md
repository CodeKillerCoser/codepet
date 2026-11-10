# OpenCode 自定义模型未出现在 Remote

## 现象与复现范围

2026-11-11，用户反馈另一台 Mac mini 的全局 OpenCode 配置包含多个自定义模型，Remote
模型选择仍只有内置模型。用户确认是前一轮 500 的同一设备；升级 OpenCode 已解决 500，
模型列表作为独立问题排查。当前未取得升级后该实例的原始模型目录，不能断言唯一根因。

## 证据与获取链路

- `crates/providers/codepet-provider-opencode/src/client.rs::list_models` 调用自有服务的
  `GET /api/model`；`list_providers` 调用 `/api/provider`，agent 调用 `/api/agent`。
  不在 Remote 解析配置文件。仅当模型列表为空时，Provider 尝试 CLI `models` 回退。
- `provider.rs::refresh_metadata` 启动后获取目录，只对成功的空结果最多重试三秒。
  三个列表均非空就立即采用，因此“已有内置模型但自定义模型尚未就绪”不会继续重试。
  后续只有 `runtime.getInstalled(refresh=true)` 再触发完整 metadata 扫描。
- `map_event` 没有处理 `model.updated`、`provider.updated`、`config.updated` 等目录失效通知。
  首次结果写入 capabilities 后不会随这些事件刷新。
- `mapper.rs::capabilities` 保留 `enabled=true`、非 deprecated、Provider 未被禁用且出现在
  Provider 列表中的模型；按 Provider 分组。没有只保留内置模型的白名单。
- Remote 的 `conversation_detail_screen.dart::_ModelSelector` 遍历下发的 modelCatalog；
  `device_session.dart` 按能力 revision 变化重新读取能力。它不内置 OpenCode 模型名称。
  新建会话使用带 Provider 前缀的 flat catalog，已有会话使用 grouped catalog。

OpenCode 官方 [API 文档](https://opencode.ai/docs/api#model) 明确说明 `/api/model` 是当前快照，
可能早于插件初始加载完成；事件接口提供 model/provider/config 更新。该语义与当前一次性读取存在缺口。

## 隔离验证

本机 `/opt/homebrew/bin/opencode --version` 为 `v2.2.6`。通过 Python 启动独立 `serve`，
CONFIG/DATA/CACHE/STATE 四个 XDG 根指向临时目录，配置虚构的本地模型服务地址和测试凭据；
只读取三个目录接口，不发送推理请求。结束时退出测试进程。

在启动后约 0.22、1、3、8、15、30 秒读取，测试自定义模型均返回 enabled=true、status=active；
总模型数在前 3 秒为 11，8 秒后为 13。证明 API 支持配置模型且目录会变化，但本次没有复现
“自定义模型初次缺失”，也没有对用户远端配置做真实请求。
脱敏统计保存在临时目录 `codepet-model-catalog-q56z38z5/summary.json`，不依赖其长期保留。
第一次诊断脚本误用旧监听输出前缀，20 秒超时；修正为实际 `server listening on` 后完成上述测量。

此前用户的原生日志包含全局配置路径以及 maas 模型的 normalization 和运行记录，证明该配置曾被
读取，但跨 run 的日志不能证明 Code Pet 当前实例已完整加载目录。旧配置字段被 normalization
忽略与整个自定义模型缺失也不能直接画等号。

## 根因与证据边界

已确认我们的适配层漏掉目录更新，启动早期不完整快照可能持续显示在 Remote。
尚需用目标实例的脱敏模型/Provider ID、enabled/status、启动后不同时间点的结果确认此次触发条件。
如果原生目录已包含自定义模型，则对照 mapper 过滤与 Host 能力 revision；若原生目录始终缺失，
再核对实际可执行文件、配置位置、dataDirectory 和 OpenCode 的配置加载错误。

另一个已确认缺口是 Provider 使用固定的能力 revision。Host 原样下发 revision，Remote 在
同代同 revision 时复用旧能力；因此只增加目录读取、不更新 revision，仍不足以刷新手机缓存。
两个缺口的最初引入提交未确认。项目配置需要单独验证 location 语义，不与本次全局配置混淆。

## 修复实现与受影响模块

- `provider.rs`：提取现有目录发现逻辑供首次探测及后续刷新共用。首次仍等待整轮 metadata
  完成再 Ready，之后同一个任务通过 watch 通知处理 model/provider/agent/config、凭据、插件和
  integration 更新。150ms 内事件合并；请求期间新事件保留，完成后补查。模型刷新不重跑 auth/stats。
- 每次目录失败保留上次成功结果，短暂错误允许有界重试；成功且内容变化才分配新 revision，
  发布既有 instance 状态通知。同内容不修改 revision，不产生额外通知。
- epoch、状态锁和任务取消共同保护停止、重启、手动刷新；旧代请求不得回写或复活 Ready。
- fixture 新增目录快照、事件和阻塞栅栏；`catalog_refresh.rs` 覆盖迟到自定义模型、create/send
  两套目录、100 条通知合并、未变化 revision、禁用及删除部分模型、在途更新、503 保留与恢复、停止重启。
- `provider_vertical.rs` 和 `stdio_framing.rs` 原先硬编码 revision，现改为使用真实返回值。
  Host 和 Remote 生产代码不变，沿用 provider.changed / 心跳摘要 → provider.describe 链路。

## 验证与剩余风险

已执行：

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-opencode -- --test-threads=1
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test builtin_provider_integration codepet_host_runs_all_sdk_based_builtin_providers_end_to_end -- --test-threads=1
# 在 codepet-remote 仓库
flutter test test/devices/device_session_test.dart test/gateway/gateway_client_test.dart --reporter expanded
```

首次扩大检查发现旧纵向测试硬编码版本号导致 stale_capability_revision；调整测试为真实协议用法后，
纵向 4 项、stdio 3 项通过。新增目录回归 3 项、原有单元 26 项、边界 1 项通过；Host 端到端 1 项、
Remote 101 项通过。原生历史/发送/启动等 4 项 opt-in 测试默认忽略，未声称目标 Mac 实机验收完成。

本次已实现刷新缺口修复，未提交、发布或安装。发行后应在目标 Mac 上检查全局自定义模型首次加载和
配置热更新：不用重连，手机能看到新增及移除；若原生 API 始终不返回这些模型，则继续核对该实例配置。
完全无可用模型时仍沿用 mapper 的发现失败策略，本次没有修改该策略。错误恢复在有限重试后依赖新的
目录事件或显式刷新，避免持续高频请求。原生接口和事件语义变化需继续通过隔离运行验证。
