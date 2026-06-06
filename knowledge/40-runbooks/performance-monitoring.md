# 性能监控基线

## 现象

用户希望在修改性能热点前先建立可复现的数据基线，覆盖内存增长、CPU 占用、后台 I/O 和日志中已有的 perf span。

## 数据来源

- `code-pet/logs/code-pet.log`：Rust 与前端通过 `app_log::record_perf_event` 写入的结构化日志。
- `scripts/export_perf_logs.py`：把日志清洗成 CSV 表，便于用 Excel、Python 或 BI 工具分析。
- 外部进程采样：后续可补充 CPU、Working Set、Private Bytes、线程数、句柄数和磁盘 I/O。

## 导出步骤

在仓库根目录执行：

```powershell
python scripts\export_perf_logs.py --log-dir "$env:LOCALAPPDATA\code-pet\logs" --include-archives --out perf-log-tables
```

如果要分析单个日志文件：

```powershell
python scripts\export_perf_logs.py --log C:\path\to\code-pet.log --out perf-log-tables
```

脚本会输出：

- `log_events.csv`：所有可解析日志行，包含时间、级别、target 和原始 message。
- `perf_events.csv`：`[perf]` 行展开后的明细，动态字段以 `field.` 前缀输出。
- `perf_summary.csv`：按 `name + status` 聚合 count、error_count、avg、p50、p95、max。
- `perf_timeseries.csv`：按时间桶聚合 perf 事件，默认 1 分钟。

## 测试场景

- 空闲基线：只打开桌宠，至少 15 分钟。预期 CPU 接近空闲，`perf_timeseries.csv` 不应显示每秒持续写入。
- 高频事件：连续触发 hook 或追加 audit line。观察 `token_usage.*`、`frontend.pet.sync_recent_events` 的 count、p95 和 max。
- 大 transcript：使用真实或构造的大 transcript 触发 token usage 刷新。关注 `duration_ms` 是否随 transcript bytes 增长，以及是否出现刷新堆积。
- 审批生命周期：触发 waiting-approval，分别测试允许、拒绝和超时。后续补充 `approvals_len` 指标后，应确认它会回落。
- Claude 运行态：连续收到同一 transcript 的 thinking/running 事件。后续补充 watcher 指标后，应确认 active watcher 不重复增长。

## 自动化测试

- `python scripts/export_perf_logs_test.py`：覆盖普通日志行、perf key-value、带空格的 quoted value、CSV 导出、summary 和 archives 发现。
- `cargo test --manifest-path src-tauri/Cargo.toml app_log_tests`：覆盖日志格式和 perf 行写入格式。

## 风险

- 现有日志只包含已埋点的 perf 事件，不能直接证明内存泄漏。内存、句柄、线程和 I/O 仍需要外部进程采样或后续新增指标。
- `perf_events.csv` 依赖日志中的 key-value 格式。如果 `app_log::format_perf_event` 变化，需要同步更新脚本测试。

## 未知项

- 当前没有进程级采样脚本，无法从日志直接得到 CPU 百分比或 Private Bytes。
- 当前没有 `approvals_len`、active watcher、AudioContext 数量等运行时计数器，需要后续 instrumentation 补齐。
