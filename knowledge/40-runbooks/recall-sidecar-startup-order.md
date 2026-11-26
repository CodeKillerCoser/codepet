# 召回 sidecar 偶发启动失败：安装、单例接管与停止时序

## 现象与证据

2026-11-27 用户反馈 sidecar 有时无法启动。以下是隔离目录中的复现，不是对某次生产失败日志的推断：

- 保持 `service.lock` 被占用，启动接入进程，再释放旧 owner 的锁。修复前 `test_waiting_attacher_takes_over_when_owner_exits_before_ready` 等待 4 秒仍没有握手；原逻辑只轮询旧 endpoint，不再尝试取得已释放的锁。
- Rust 启动锁保持超过一秒后调用 stop，原 `stop_is_not_lost_while_startup_holds_the_process_lock` 失败：停止函数已返回，稍后创建的 Process 仍保留。
- App setup 在后台安装之前启动 recall watch；watch 可以读取旧 `current.json` 或尚未完整安装的组件。原服务在启动 HTTP 线程、进入事件循环之前就发布 runtime 和握手；非 owner 仅检查 protocol=1，可以附着不支持新功能的旧版本。

## 根因与引入历史

“安装完成”“进程就绪”“单例所有权”“进程退出”原先没有统一的先后约束。接入分支只选择一次，停止又把“未及时拿到锁”当作可以放弃。具体引入提交未确认。

## 修复与涉及模块

- `http/runtime.rs`、`recall/preparation.rs`、`src-tauri/src/lib.rs`：注册安装结果屏障后才启动 watch。HTTP/账本先独立可用；召回请求等安装完成，失败传递明确原因，不读取升级前的 manifest。
- `recall/mod.rs`：安装和启动等待支持取消；stop 先取消启动，再取得生命周期锁，不能静默丢弃。拿锁后重读模型设置，防止排队请求启动旧配置。首次目录尚未创建时从已有父目录解析单例 key，避免目录创建后换成另一个 Recall、绕过安装屏障。
- `recall/process.rs`：显式停止发送 kill 后等待子进程退出，再释放 Gateway、允许替代进程启动。配置写入、握手、接入进程退出和健康确认各有截止时间；仅启动阶段连接失败重试一次，不重放业务写入。取消期间仍有 kill_on_drop 兜底。
- `codepet_recall/lifecycle.py`、`server.py`：每轮等待都重新竞争 OS 锁；先进入事件循环、启动 HTTP，再发布 runtime 与握手。复用同时验证协议、配置 fingerprint 和不可变 bundle manifest 摘要。活着的不兼容 owner 不会被另一 Host 杀死，超时给出关闭旧 App/重连 Agent 的明确原因。

## 验证结果与回归防线

本机 macOS：修复后 Python 全套 35 项通过，包含八进程同时启动仅一个 owner、旧 owner 就绪前退出后的接管、延迟事件循环不能提前握手、不兼容服务不得复用，以及既有鉴权、父进程 EOF、队列恢复与图缓存测试。25 轮八进程并发启动（200 次进程启动）全部通过。Rust 覆盖安装屏障、MCP 配置独立可用、安装等待取消、慢启动期间 stop 不丢失、目录创建前后单例一致，以及健康 409 不重启加工进程。使用已安装 CPython 和当前源码的独立测试包，实际通过十轮启动→附着→停止→立即重启；没有修改正式索引或调用模型。Tauri lib 编译与源码大小检查通过。

最小检查（无需 Cognee 或模型）：

```sh
PYTHONPATH=sidecars/task-recall python3 -m unittest discover -s sidecars/task-recall/tests -p test_lifecycle.py -v
PYTHONPATH=sidecars/task-recall python3 -m unittest discover -s sidecars/task-recall/tests -p test_service.py -v
cargo test --manifest-path crates/Cargo.toml -p codepet-task-mcp --lib
npm run check:source-size
```

`task-recall-quality.yml` 在 Linux/Windows 执行这些启动测试和 Rust 单测。可选实际进程测试：把含当前源码、manifest 和独立 CPython 的**隔离包**路径传给 `CODEPET_RECALL_LIFECYCLE_BUNDLE`，运行 `native_start_attach_and_immediate_restart_release_the_owner_lock -- --ignored`。不要改已安装不可变包来拼测试环境。

## 风险、限制与未知项

用户当次失败没有对应日志，不能认定以上覆盖了全部偶发退出原因。Windows 本轮只有 CI 回归入口，本机未执行。需要重新打包 Rust 与 Python sidecar，替换前端不能生效；本轮尚未替换已安装 App。旧 stdio owner 仍活着时需重连持有者，不会为抢占版本中断其他 Host 的加工。父 owner 退出后，已附着 Host 的在途请求可能失败，后续请求清除失效连接再建立服务；不自动重放已发出的写操作。

任务 MCP 读取已有 Run 返回“任务格式版本或执行历史无效”，本轮无法更新该回执；保留此证据，不绕过账本校验直接改数据。任务账本错误不属于本轮启动修复。

## 规约候选

已补充 `knowledge/60-rules/local-task-mcp-transport.md` 的 sidecar 生命周期约束；基础构建和路径检查继续参考 [sidecar runbook](task-run-recall-sidecar.md)。
