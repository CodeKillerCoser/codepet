# 任务 MCP 本机性能与超时回执

## 现象与证据

MCP 使用反馈记录了多次写入超时或迟迟无回执。一次本地验收重新查询后确认已写入，最后一次归档遇到 `Task lineage is busy in another operation`，现场结果未确认。

本机为 Apple M5 Pro、macOS 27.0。用 `9dbc7b12` Release stdio 程序、隔离账本和 1,200 文件的非快进 merge 跑三轮。带 merge 的更新最大 4,584.50 ms，关联后追加评论最大 4,508.03 ms，完成写入最大 5,669.73 ms。精确查询最大回包 5,162 字节。夹具重现了慢写入与重复 Git 明细，现场的分钟级耗时原因未确认。

## 根因与涉及模块

- `tasks/diffs` 原先逐文件启动 Git。超过 300 文件产生的截断标记会使后续写入再次采集该提交。批量读取 NUL 分隔的原始路径与同序补丁，保留已有归档，显式读取旧记录可补齐未采集的提交。
- `store.rs` 将 JSON 序列化输出直接写入临时文件。仅批量 Git 后轮次最大值仍为 2,025.95 ms。加缓冲后同一夹具的八轮最大耗时降至 167.33 ms。刷新缓冲后仍同步临时文件并原子替换，回执文件在数据之后发布。
- `tasks/find.rs` 和 MCP 路由将摘要和明细分开。默认验证结果按计划、阶段、目标和环境归并。活动和 Git 文件分页。
- 扩到 20 轮后，召回后台在 sidecar 缺失时仍扫描完整账本，前台发布遇到一次 busy。后台先确认可用性，列表读取在锁内取得一致的原始文件，把 JSON 解析移到锁外。召回查询使用已启动的 sidecar，前台等待最多 60 ms，未就绪或超时走本机关键词查询。
- `workbench.rs` 原先每个 Git 进程最多多等一个 10 ms 的轮询间隔。三次 merge 一起登记时，五轮最大值为 210.70 ms。缩短初始轮询间隔后，十轮最大值为 152.55 ms。
- 验收模型明确区分清单项的适用范围与一次提交的阶段。服务端拒绝跨阶段验收项，并返回验收项 ID 及提交阶段。

## 复测与处理

在 macOS 本机用以下命令复测真实 stdio。脚本创建临时 HOME、Git 仓库和账本，统计请求发出至完整 JSON-RPC 回包的最大耗时和字节数。

```sh
cargo build --manifest-path crates/Cargo.toml --locked -p codepet-task-mcp --release
python3 scripts/task_mcp_latency.py crates/target/release/codepet-task-mcp --samples 20 --budget-ms 200
python3 scripts/task_mcp_latency.py crates/target/release/codepet-task-mcp --samples 10 --merge-sizes 1530 1048 1072 --budget-ms 200
```

最终覆盖八个工具、千文件提交文件分页与前 300 份补丁归档。单 merge 20 轮共 362 次调用，最大 84.51 ms，完成写入最大 81.73 ms，精确查询最大 1.17 ms，回包最大 1,104 字节。三 merge 10 轮共 182 次调用，最大 152.55 ms。两组持锁回执查询均低于 0.3 ms，普通查询约 40 ms 返回 busy。原始报告位于 `artifacts/task-mcp-latency/`。

2026-11-28 账本正文外置后用同样的 Release 命令复跑。单 merge 20 轮最大 97.27 ms，三 merge 10 轮最大 185.81 ms，544 次调用均符合本机 200 ms 门槛。脚本同时核对任务清单和补丁记录中的相对引用，并从补丁文档读取首个文件的实际内容。测试报告为 `documents-single-merge.json` 和 `documents-three-merges.json`。外置布局回归覆盖旧内嵌记录、内容去重、相对引用范围、事务回放和移动账本。122 项 Rust 测试、stdio 与 HTTP 黑盒、`npm run check:source-size` 和 `git diff --check` 通过。存储路径和搬迁范围见 [Task 与 TaskRun 对象定义](../30-domains/task-management/task-record-model.md)。

发生超时后，用同一来源和原 `operationId` 调用 `get-task-operation`。收到 `committed` 后使用其中的 revision。若为 `unconfirmed`，保留原键和完整请求以便做幂等重试。继续查整个任务无法确认某次具体写入。

Rust 账本与 MCP 回归用 `cargo test --manifest-path crates/Cargo.toml --locked -p codepet-task-mcp -p codepet-task-lineage --release -- --test-threads=1`。HTTP 并行套件曾遇到临时端口争用，串行复跑通过。stdio 与 HTTP 黑盒使用两份隔离账本，覆盖重启幂等、回执权限与查询拆分。真实模型和原生发行包另需部署后的环境。

200 ms 验收衡量本机夹具的端到端响应。锁争用可能快速返回 busy，真实环境的磁盘挂起、冷启动迁移和更大历史账本仍会影响写入完成时间。
