# 任务存储锁必须由操作生命周期显式释放

## 规则

Store 与抽取 lease 的 Drop 必须显式 unlock。不能只依赖关闭一个 File 来结束文件锁生命周期。

## 适用场景

App、独立 MCP、Git 子进程并行时的任务账本与旧抽取 Store。

## 反例

锁文件描述符仍有副本时，关闭原描述符可能没有释放底层锁；紧接着的独立打开会返回 busy。依赖立即重新获取锁的旧抽取测试在并行执行时出现该失败。子进程启动前后的描述符继承窗口是可能触发因素，未直接抓取子进程 FD 证明。

## 推荐做法

以 RAII 包装明确锁的拥有者，在 Drop 中先调用 fs2 unlock，再由 File 关闭句柄。共享领域服务继续短时持锁，Git 查询在锁外执行，写入在锁内检查 revision 并原子提交。

任务账本的 JSON 原子写入必须用缓冲区合并序列化产生的小块写入。按写缓冲、flush、同步文件、原子替换的顺序提交，回执放在任务和证据之后。不可变、原子发布的回执允许在另一写入持锁时直接读取。MCP 等待独占锁的预算为 40 毫秒，超出时报告争用。千文件 merge 的 stdio 对照和回执验证见 [任务 MCP 本机性能](../40-runbooks/task-mcp-local-performance.md)。

## 来源

2026-11-24 任务 MCP 首版集成；`store.rs::operation_releases_locks_even_when_a_duplicate_descriptor_is_retained` 用 try_clone 保留同一文件描述符的副本，覆盖原拥有者结束后仍能重新获取锁。

## 验证方式

运行 codepet-task-lineage 和 codepet-task-mcp 的默认并行测试，覆盖 lease、writer、事务恢复、并发写与幂等。不得靠固定等待或仅改串行运行掩盖锁未释放。
