# 最近会话不支持与列表反复刷新

## 现象与证据

2026-09-09，用户报告 Remote 的 Codex 最近会话不支持拉取，首页和消息页反复刷新。桌面启动日志为 `0.3.9-beta+2bfd766`；Remote 从 `01f8af9` 构建并覆盖安装到本机 Android 16 模拟器，保留配对数据。模拟器连接恢复后，Codex 显示连接中，最近区域显示“当前 Gateway / Provider 不支持最近会话”。此截图只能证明能力未就绪，不能单独确定原因。

本机 `logs/events.jsonl` 在 13:44:01—13:44:21 每约 5 秒记录一次 Codex Ready 状态通知；revision 的 observations 后缀为 19、21、23、25、27，均缺少 `conversation.active.list`。该轮日志包含 1579 条 Codex 摘要事件、仅 257 个不同会话，表明相同扫描前缀反复发布。Provider 日志没有输出扫描批次提交错误；不能仅凭缺少错误日志认为提交成功。

回归测试 `usage_sink_preserves_batch_admission_and_retry` 向用量包装层发送 600 条事件，在修改前因逐条调用下游 `publish` 失败，修改后整批接纳、拒绝后无部分输出以及恢复后的顺序检查均通过。

## 根因与引入历史

`06ffe4f` 新增 `UsageSink`，只实现 `publish`，继承了 trait 的逐条 `publish_batch` 默认实现。它包在三个 Provider 的生产事件出口上，把原本原子接纳的摘要批次拆成单条队列条目。超过 256 条且消费者来不及排空时，前缀已经发出，后续入队失败，批末的 Ready 能力通知无法发出。

Codex observer 将提交失败转为未就绪，增加能力 revision；成功基线未安装，5 秒后再次提交完整扫描。因此最近能力持续缺失，重复摘要和能力通知又触发 Remote 列表投影、描述加载和详情绑定重载。该链路解释日志中的周期性重试；正常 Hook 活动产生的实时刷新应继续保留。

## 修复与涉及模块

- `codepet-provider-data/src/collection.rs`：将采集抽成共享辅助函数，单条和批量入口都保留用量采集；批量只调用一次下游 `publish_batch`。
- `codepet-provider-data/tests/collection.rs`：覆盖三个 Provider 的 600 条整批接纳、拒绝和重试；OpenCode 批量重复通知验证用量仍可持久化且不重复计数。
- Codex observer 与 Host Gateway：分别控制完整扫描就绪和最近能力聚合，是故障的传播路径，本次未改变能力判定。
- Remote 的 DeviceSession、最近和详情控制器：消费上述事件；本次未通过屏蔽事件或缓存能力掩盖 Host 错误。

## 验证结果

`cargo test --manifest-path crates/Cargo.toml -p codepet-provider-data`：28 项通过。调整批量采集样本后 collection 6 项再次通过。构建前 `npm run protocol:check`：20 项通过。

Codex `provider_vertical` 全量：49 通过、3 失败、1 忽略。两项 EOF 测试单独重跑通过；`provider_v1_round_trips_fixture_app_server_lifecycle_and_approval` 单独仍在“不包含 data:image/png”的断言失败，该失败尚未修复，不能宣称完整回归通过。

桌面使用 `npm run tauri build -- --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'` 构建，覆盖 `/Applications/Code Pet.app` 后重启；签名校验通过。安装后的桌面可执行文件与 Codex Provider 分别与构建产物 SHA-256 一致，三个 Provider 进程均从新安装包加载。旧包备份为 `~/Documents/Codex/CodePet-builds/Code Pet-before-recent-fix-20260909.app`。

14:29:54，Codex 完成扫描并发布 `observations-2`，包含 `conversation.active.list`。持续观察到 14:32，未再发布能力撤销或 revision 递增事件；后续摘要只涉及少数真实活动会话，未再重复发送 257 条扫描前缀。模拟器 Codex 显示在线，“最近”成功显示 `20+` 条记录。进入历史会话并向上滚动，间隔约一分钟的 UI 层级中 20 个可见元素的内容与坐标完全一致；期间本次排查会话仍有 Hook 事件。这验证了该历史页在本场景下未反复重载或跳动，不等于所有消息与滚动场景都已覆盖。

返回首页后再次跨多个扫描周期比较，20 个可见元素的标题与坐标保持一致；14:33:44 仍未出现新的 Codex 能力变化。正常活动的时间标签仍可变化。

Remote 已完成 debug/release 构建，使用本机既有正式签名覆盖安装，最终调试包保留原包名和配对数据。复现截图保存在 `~/Documents/Codex/CodePet-builds/recent-fix-20260909/`。未执行格式化工具。

## 回归防线与排查步骤

1. 核对已安装桌面与 Remote 的版本，区分生成 SDK 解码错误、网络未连接和能力确实未就绪。
2. 按 Provider 与时间聚合摘要身份和能力 revision；发现每约 5 秒重复同一前缀时，检查扫描提交到 stdout 队列之间每层 sink 的批量方法。
3. 用超过 256 条的批次测试整条包装链，覆盖队列拒绝和恢复；不要只测底层队列或无界闭包 sink。
4. 安装修复后，观察多轮扫描中最近能力是否稳定、历史消息页是否保留滚动位置，同时确认真实活动仍能更新。

规约见 [事件包装层的批量接纳](../60-rules/provider-event-batch-admission.md)。真实设备、其他操作系统和持续高负载仍需独立验证。
