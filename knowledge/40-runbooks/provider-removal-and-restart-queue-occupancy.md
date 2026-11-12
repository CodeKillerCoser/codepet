# 自定义 Provider 移除与重启后空闲队列卡住

## 现象与证据

2026-11-12 用户报告本机运行时只有添加自定义 Provider 的入口；另报告任务执行期间退出桌面 App，重启后手机显示空闲，新消息却停在排队状态。

界面原先只有 import 命令，没有 remove。Host `input_queue.rs` 的 accepted 响应会持久化 `input_queue_runtime.active_turn_id`，`head()` 在这个字段非空时直接返回，无机会调用 Provider 查询当前会话。原启动恢复只处理 queued/dispatching/steering 等输入，未清除旧会话占用。新增回归在受理 running turn 后重新打开数据库，再提交 idle 会话的新输入，可复现旧标记阻止选取队头。用户现场具体会话尚未直接验收，不能把所有排队都归为这个原因。

## 根因与修复

运行占用跨 Host 生命周期持久化后，被当作当前进程的运行事实。引入提交未确认。启动时事务内清除旧运行占用；派发仍经现有 conversation.get 校验真实 active turn。accepted 输入不重发；dispatching/steering 仍转 outcomeUnknown，启动时已有待发项暂不自动重放；用户正常发送新消息就是继续处理的显式意图，Host 在同一事务内解除安全队列的暂停并保持 FIFO，无需额外“继续队列”按钮。若旧项是 blocked/outcomeUnknown，新发送直接返回 input_queue_requires_attention，保留草稿并引导核对移除旧记录，不能假装入队成功后永远等待。不会对“继续执行”做特殊文字解析，任意正常发送都遵守同一语义。

自定义 Provider 的移除在 Host manager 内复用导入锁和该插件操作锁：校验包属于受管安装目录、停止进程、归档至兄弟目录 removed-providers、从活动目录和实例列表移除、发布状态变化。只允许移除受管导入包，拒绝内置包、外部搜索目录和路径逃逸。文件归档与数据库/会话保留，重新选择归档包 manifest 可恢复；不会清空用户历史。原目录导入也可以重新安装。

## 涉及模块

- `providers/manager/import.rs`：停止、包路径约束、归档、注册表更新及失败回滚。
- Tauri bridge、API、`App.svelte` 与 `ProviderRuntimeInfo.svelte`：原生确认框、忙碌状态、自定义项移除入口、列表刷新、完成后焦点返回添加入口。
- `webcontent-contract.json`：backend API 12 → 13，避免旧原生程序加载缺少 remove 命令的新 UI。
- `input_queue.rs`：启动占用失效处理，不更改已受理和结果未知输入的持久化语义。

## 验证结果与回归防线

- Host 队列存储 9 项通过，含新增 accepted-running → reopen → idle 新输入可派发，以及新发送恢复有待发项/无待发项的暂停队列、未知结果不重试且新请求明确拒绝。
- manager_gateway 的 import 模块 5 项通过，含真实子进程移除、保留包资源、拒绝其他来源、重新导入与其他 Provider generation 保持；Gateway queue 模块 4 项通过。
- Vitest 35 项通过，覆盖移除动作、内置项无动作和 busy 禁用；`npm run build` 与 Tauri lib check 通过。
- 模拟 IPC 的浏览器页面检查布局、键盘导航、添加后移除及列表提示；不操作个人 Provider。这不替代用户现场原生确认框及手机重启验收。

桌面 App 完整打包成功，产物为 `/Users/wangxin/Documents/CodePet-builds/provider-remove-restart-20261112/CodePet-macOS-arm64-v2.zip`；本地签名校验通过，未公证。未替换运行中的已安装 App，未操作真实队列。更新后的构建日志 `/tmp/codepet-queue-continue-app-build.log`。

持久化的运行占用必须在新 Host 启动时失效，不能直接解释为新连接的活动任务；派发前仍须读取 Provider 当前状态。对结果未知输入的自动重放会造成重复执行，禁止以清空账本或强制解除暂停修复。

## Echo 测试插件收尾

正常数据目录 versions/providers 下发现 independent.echo.jsonl、independent.echo.zstd、independent.echo.mux 三个前期验收安装包，它们的握手展示名称相同而 manifest 名称不同，造成重复展示的观感；不是内置 Provider 或结构解码错误。三个包已移至 versions/removed-providers 中带 qa-cleanup 的独立目录。未删除数据库或历史，Pi 和内置插件保持原状；未重启运行中的 App，现有进程列表要到下次重启才更新。这是测试安装残留，后续验收仅使用隔离数据目录。
