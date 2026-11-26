# 图谱冷启动：快照先展示与后台补生成

## 现象与证据

用户安装 de3ae9f 后仍反馈首次慢。正式日志中组件准备约 8.8 秒；同一 510 节点、1,270 边索引副本去掉预览缓存，首次基础图导出 3.52 秒，而已有快照的 HTTP 读取约 23ms。先前缓存测速不能代替首次安装到首屏的验收。

## 根因与引入历史

`Recall::graph_preview_since` 原先统一经过安装屏障、Python 启动和 HTTP，磁盘已有成功快照也必须等待。Python 仅在加工或页面请求时导出，旧索引没有新任务时不会主动发布预览。de3ae9f 已解决连续加工时的发布，尚未把安装等待移出首屏路径。

## 修复与模块

- Rust `recall/preview.rs` 只读 `published/preview-graph.json`，在阻塞线程中解析，不打开原生数据库。按当前模型配置 fingerprint、dataset、格式版本及非空 previewVersion 校验，读取上限 32MiB；配置和 dataset 在读取后复核。不得读取 writer 工作缓存，也不修改索引或账本。
- `Recall` 首次基础图优先返回快照，安装未完成时支持相同版本的小型回执；没有快照则立即返回准备中。其他业务请求仍遵守安装屏障，取消逻辑保留。下一次读取回到 live service，只有服务提供实际队列数量，Host 不伪造完成/排队计数。
- Python 服务先发布 ready 握手和 HTTP，再启动工作线程。旧索引缺快照时，在写锁内主动导出基础图；已有快照不加载原生依赖。此路径不加工文档、不请求模型，模型加工关闭时也可读取既有索引。失败不重跑模型，后续正常读取可重试；Semantic 保持按需计算。
- 前端已有图始终可用，连接后台时每秒检查，就绪后恢复五秒检查；同版本只更新连接状态。`task_graph.first_paint` 在渲染器就绪后两个动画帧记录，`task_graph.components_ready` 单独记录安装准备，字段仅含耗时、来源和节点/边数量。

## 实测结果

macOS arm64，本地 debug 原生 App、正式数据的已发布快照。主界面时间取进程启动日志到 frontend.main.refresh，图谱时间取进入页面到两个绘制帧；不是单独的 JSON 读取时间。

| 场景 | 主界面初始化 | 图谱首屏 | 组件准备 |
| --- | ---: | ---: | ---: |
| 本地测试包首次运行 | 1.738s | 251ms | 43.325s |
| 同包再次运行 | 1.655s | 243ms | 2.478s |

首次使用未压缩组件和 debug 校验，43.325s 不能与正式发行压缩包的 8.8s 作安装速度对比；本轮证明首屏不再依赖它。首份图在组件准备完成之前显示，截图人工确认节点与连线可见。Rust 独立实测在安装永久未就绪的条件下读真实快照约 72ms。

另用隔离的真实索引副本清除工作/发布快照：Python 握手 166ms，未请求 `/graph` 的情况下 2.89s 自动发布，随后读取约 18.7ms。模型端点设为不可达回环地址，未运行抽取。以上原始日志、JSON 在 `artifacts/graph-performance/cold-start/`。

## 验证与回归防线

Rust 21 项通过、5 项显式集成测试忽略；新增安装/启动锁阻塞仍可读、无快照快速准备、同版本交接及格式/配置/dataset/大小边界。Python 38 项通过，覆盖握手先行、关闭加工时后台补生成、原生写入隔离及原有生命周期。前端 11 项通过，覆盖连接状态撤销不重绘、自动首图、交互保护和卸载。

前端类型检查、生产构建、源码尺寸检查及原生 CLI 构建通过。直接 cargo build 未正确应用 macOS Tauri feature，改用项目的 `tauri build --debug --no-bundle --config src-tauri/tauri.bin.conf.json` 完成；CLI 自动改动的 Cargo feature 未纳入业务提交。

读正式快照测试可用 `CODEPET_RECALL_PREVIEW_PROBE_DATA=<活动数据目录> cargo test --manifest-path crates/Cargo.toml -p codepet-task-mcp real_published_preview_does_not_wait_for_installation -- --ignored --nocapture`，不要删正式缓存模拟冷启动。后台补生成使用索引副本，关闭模型端点且仅观察发布文件，不发送 `/graph`。

## 风险与未知项

快照可能早于最新索引，页面明确显示上次结果并向服务接续；检索证据仍由原生哈希校验。格式版本修改须同步 Rust/Python 契约。实机另有 Agent 持有旧 bundle owner，按既有规则没有抢占；live 交接由受控 HTTP 回归验证。测试结束恢复正式 App 与安装记录，没有杀死其他 Agent 的服务。

这是已有数据升级和缺快照旧索引的优化；全新空数据仍须完成首次加工。没有声称已减少发行包解压成本，依赖包拆分不在此次范围。Windows 和 CI 正式发行包的安装后首屏仍待验收，本地 debug 测量不能当成发行包承诺。约束同步到本机任务传输规约。
