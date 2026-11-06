# 2026-11-06 手机跨网连接失败：TURN 预留容量不足

## 现象

手机完成自动登记后，切换公网连接仍失败。用户提供 `codepet-logs-2026-11-06T13-11-33-580353Z/`，记录结束于北京时间 21:11:33。导出只有 JSON，RTC 事件又嵌套在 message 字符串中，重复 ICE 快照掩盖了关键路径。

## 证据

手机共 16,812 条记录，无损解析；以下时间均为北京时间（手机/VPS UTC 加 8 小时，coturn 日志 -0500 加 13 小时）。

- 21:09:11，当前 Mac（device-8d247c27…）通过 LAN 选中连接，约 151 ms；21:09:55 手机 `signal.registration.complete`。这次不是登记失败。
- 21:10:22 原 LAN WebSocket 关闭，之后 LAN 8 秒超时。用户本次未明确说明切网操作，不能仅凭此证明关闭原因。
- 当前 Mac 的公网尝试：IPv6 在约 129 ms 后 `noCandidates`，只有私有 IPv4 和回环候选；这段记录不支持声称手机具备可用 IPv6。
- 21:10:24 发起的当前 Mac relay 路径，在 40,764 ms 后没有中继候选。coturn 在 21:11:12 对同一 Mac/credential 的 UDP/TCP/TLS 申请反复返回 `486 Allocation Bandwidth Quota Reached`。
- 21:11:08 VPS 收到当前 Mac 的 IPv4 offer，21:11:09.403 电脑完成签名校验，21:11:09.922 VPS 收到 answer。手机 21:11:11.216 重新扫码配对成功，随即替换旧 session，因此取消了该尝试，未取得 answer；其后记录成 answerTimeout，不能据此断言电脑没回复或配对失败。
- 下一次当前 Mac offer 在 21:11:54.758 到 VPS，21:11:58.306 answer 已交给手机；Host 最终仍未打开 DataChannel。该段晚于手机导出截止，只能用服务端/电脑证据补充，不能虚构手机侧状态。
- 手机同时自动重试另一个 Host（device-544a159c…）。此 Host 在窗口内无应答。它在 21:09:14 已占用 5 条 TURN 分配，后续重试持续竞争配额；不能把这些 offer 当作当前 Mac 的请求。

## 根因

旧部署 `max-bps=2000000`、`bps-capacity=10000000`，同时 `total-quota=64`。coturn 的 bps-capacity 是各 session 可分配带宽预算，非单纯观测当前流量的上限；5 条分配已耗尽预留，即使尚未传输业务数据，后来的请求也可能被拒绝。多网卡/多协议候选和旧设备重试使现有配置不足，当前 Mac 的中继兜底因 486 被阻断。

依据：[coturn 官方配置说明](https://github.com/coturn/coturn/blob/master/examples/etc/turnserver.conf)。现场 allocation count=5 与 486 对应，是本次明确根因。IPv4 没建通、IPv6 无候选与旧 Host 不响应是各路独立失败，不能统一归因于“信令不在线”。手机还有约 40 秒等待完整 ICE 收集的问题，会压缩 55 秒总预算；本轮不以盲目截断收集代替正确的候选处理。

## 引入历史

配额来自仓库 `services/signaling/deploy.py` 的初始部署参数；具体首次引入提交未确认。此次并行连接及多设备后台重试暴露了预留容量与并发数量不一致。

## 修复与涉及模块

- `services/signaling/deploy.py`、`deploy_turn_capacity.py`：保留总容量 10,000,000 B/s，将单分配上限调整为 500,000 B/s（约 4 Mbit/s）、总分配数 20，单用户仍为 8。两项乘积与容量一致，支持两个用户各 8 条分配及余量。单分配峰值比旧值降低，后续如需大文件高吞吐应按 VPS 实际带宽统一调整；不取消总量、用户数、认证或私网访问限制。
- `probe_turn_capacity.py`：在服务器生成短期测试身份，持有 12 条认证分配，并在 finally 释放；不输出密钥或口令。
- Remote `diagnostic_report.dart`/`local_log_store.dart`：导出增加 `connection-summary.txt`，`codepet.log` 改为 UTC 有序文本时间线，完整候选/统计保留在 `raw/*.jsonl`；兼容旧嵌套日志。快照与写盘/轮转排队，避免导出时文件被重命名。
- Remote `AppLog`/`RtcDiagnostics`：RTC 使用单层结构化字段；状态 INFO，候选统计 FINE，失败 WARNING。增加构建 SHA、模式、平台、时区，Android CI 注入实际 SHA。
- Remote transport：主动 close 的建连尾部异常记为 `connect.cancelled`，原生 abort 仍为失败；不改通道选择、候选或超时策略。

## 验证结果

VPS 已于 21:17:39 更新，备份 `/opt/codepet-signal/backups/turn-capacity-20261106T131739Z`，仅重启 coturn。12 条同时认证分配（两个用户）通过并全部释放；真实 Rust relay 探针完成自动登记、信令、选中 relay、Gateway 握手、72 KB RPC、撤销关闭。该结果不能替代用户手机的实际网络复测。

Python `unittest discover -s services/signaling -p 'test_*.py'` 16 项通过；Remote `flutter test --no-pub test/diagnostics test/gateway test/settings test/app test/pairing` 164 项通过，涵盖旧日志解码、排序、取消与失败分离、导出内容及日志分享入口；相关 Dart 分析通过。当前用户日志已通过生产转换器生成可读时间线/摘要，原目录未修改。转换验收还发现历史消息含 NUL/控制字符，已统一替换以避免文本被识别成二进制；相关新增回归及导出测试 7 项通过，原始 JSONL 仍完整保留。

交付：Remote `5947136` 已由 GitHub Actions run `42226855471` 构建正式签名 Release，包内构建 SHA 与提交一致，无 debuggable 标记。安装包已部署 VPS 下载入口，完整下载后 SHA-256 与构建产物一致：`e8166c71c12fbb599ebc21cc6871c3e05170e1bd17ce2f7b0203cd7b6a55c854`。本次服务器修复无需更新手机即可复测；新版手机用于改进日志导出和取消状态记录，未改桌面二进制。

## 回归防线与未知项

容量配置必须满足 `total-quota × max-bps <= bps-capacity`；验收至少包含两个用户并发分配，不能仅跑一个中继连接。后续诊断先按目标身份/race/offer 对齐，再读全量 ICE 明细。缺少候选不能直接推断服务端 486，必须有 coturn 证据。

手机真实网络修复后能否连通待复测；没有完整的新手机日志，不能确认所有 NAT/代理场景。旧设备是否应删除由用户决定，本轮未删除。约 40 秒候选收集延迟仍存在，后续应按候选可用性及迟到候选策略单独修复。
