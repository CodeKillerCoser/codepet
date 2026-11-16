# Notice 发布、状态和动作

状态：2026-11-16，已实现并通过隔离环境联调。Provider 与 Gateway 业务协议统一为 version/minVersion = 4/4；公开 SDK 目录仍为 v2，安装 Manifest 为 v2，三种 STDIO profile 保持 v1。此次不保留旧 Hook/Pet Gateway 的成功兼容路径。

## 背景与边界

旧实现把原始 Hook 放进 `event.notification`，由 Host 按来源字段解释并投影成桌宠任务。这使通知助手也必须模拟 Agent Hook，状态、窗口动作和显示模型分散在多层。现在发布单元是 Notice：接入方定义状态、提供内容和可用动作，Host 管理身份、持久化、修订和路由。

唯一事实是 `protocol/v2/` 的 IDL：

- `common/notice.schema.json` 定义中立的 Notice、状态、内容、进度、动作与执行回执；Provider、Gateway 共享对象结构，分别编码。
- `provider/notice/` 定义插件主动调用 Host 的 create/update/finish，以及 Host 调用插件的 observe/action.execute。
- `gateway/notice/` 定义桌宠查询、来源启停、动作执行与 notice.changed。当前只开放给可信本地入口；远程会话不能调用这些方法，重放和实时事件均过滤 Notice 内容。

生成器同时输出 Rust/TypeScript/Dart 类型、方法方向、客户端与 dispatcher。Host 的标准 JSON Schema 校验器使用从同一 common IDL 生成的内嵌 Schema；Manifest 也使用公开的标准 Schema。约束不能另写一份到业务代码或导出脚本。涉及跨对象的归属、修订、唯一动作 ID 和幂等性由运行时检查。

## 生命周期与交互

插件进程由既有 Registry 启动，通过既有 STDIO 完成 Initialize。声明 observation 的插件可以只有通知能力，instances/instanceKinds 均可为空。Host 在初始化后发送 notice.observe，按来源保存开关；重启按新进程代次重新配置，失败会有界重试，不把配置失败显示成已连接。

Provider 收到原始来源事件后，以稳定 creationKey 和 sourceId 创建 Notice。Host 生成 noticeId 并先落库再返回；重复创建取得已有条目。后续 update/finish 携带 noticeId、operationId、expectedRevision，成功响应返回新 revision。字段组出现时整体替换；未出现时保留。finish 必须提供最终 status；再次 finish 可以更正最终内容或动作，update 不能重新打开结束条目。

业务状态 value/label 完全由 Provider 定义。Host 不把 running、failed、cancelled 等字符串转换成生命周期。finished 只由显式 finish 设置；断开、禁用来源、进程退出不构造业务最终状态。UI 按 finished 分组，直接显示 status.label 和内容/进度；本地隐藏不等于结束或删除来源条目。

动作只渲染 enabled 的已知 reply、activate、permission。回复要求用户输入，权限要求明确选择；两者由所属 Provider 执行。activate 只携带结构化应用标识或 URI，交给 Host 平台 adapter，不接受 shell 命令。系统接受启动请求只返回 accepted，不假称窗口已经聚焦。

Host 执行动作前检查所有者、来源可用性、当前 revision、动作是否仍启用及输入类型。executionId 在副作用前写入持久化回执；重复 ID 返回原结果，不再调用 Provider。进程中断、超时或异常结果保持 unknown，不自动重试。桌宠同一修订上已受理、成功或未知的动作被禁用；新修订提供新的交互意图。打开回复/权限编辑器时，窗口才请求键盘焦点。

## 实现与清理

`codepet-observation` 负责本机 Hook 安装、授权令牌、快速有界收包及 Provider 内的来源解释。HTTP 接收不等待 Host RPC；队列溢出标记独立 gap，保留原始 payload。Codex/Claude Hook 与 OpenCode 事件归并成 Notice，保留轮次替换、迟到事件和子代理过滤；工具失败不代表父轮次结束。纯 Hook 只有观察能力，有真实应用标识时可提供 activate，不制造回复或授权能力。

Codex 的会话活动投影和三个 Provider 的用量采集继续使用进程内 observer，不再借用跨进程原始通知。内置 Rust helper 通过现有 event sink 调用 Host；这只是内部复用接口，不进入外部插件交付物。

Host `notices` 模块使用现有 app-state SQLite，维护条目、发布幂等记录、动作回执及来源偏好。来源 ID 从当前已初始化 Registry 派生，不硬编码三个内置来源。卸载等待进程退出、清理连接与来源状态，并删除该插件的 Notice/偏好；保留动作执行回执以阻止副作用重放。桌宠同步剔除已卸载来源，旧快照不能恢复其条目。

STDIO 运输层只增加通用入站请求/出站响应衔接，业务分发留在 Manager。每个 Provider 独立接收循环、有界请求并发和背压；数据库操作使用 blocking worker，不持有全局 Provider 锁等待业务。mux 响应必须正常结束子流，不能写完后直接丢弃引发 reset。请求与响应的校验类型必须解析到实际所属包，不能给 common 回执错误加上 provider 前缀。

已移除 Host 旧 pet_gateway 投影、私有 pet_gateway_request/Pet SDK、Provider 原始 subscription 模块，以及未启动的旧本机 Collector、Hook 安装器和基于旧活动事件的回复/授权/激活入口。桌宠保留展示与窗口状态，统一使用标准 Gateway RPC/event emit。任务谱系的历史抽取/数据模型不在本次调整范围。

## 验证与限制

验证证据包括 NoticeStore 的持久化、归属、修订与动作回执测试；三个 profile 的独立 JS 子进程发布/动作并行测试；三个内置 Rust Provider 在零 Agent 实例下通过真实 HTTP Hook 和 STDIO 发布；远程 Notice 方法与事件隔离；桌宠悬浮/停靠界面的动作和焦点验收。独立插件作者只收到公开 Schema/README，源码及七个包各 34 项通过。

桌面 App 与 Android 模拟器运行新构建，生产配对、LAN/WSS、原生 WebRTC、心跳、三 profile 的发送/历史/队列与手机生产界面的更早消息和切会话隔离联合测试通过。可复跑步骤及产物在 `knowledge/40-runbooks/desktop-android-protocol-joint-test.md`。

未接入真实钉钉/飞书账号，也没有验证其外部权限执行；独立通知 fixture 的权限/回复仅为本地回环。Hook 来源缺少原生轮次 ID 时，发布关联使用本进程维护的轮次边界；进程中断后不会凭空推定旧条目已完成。有限队列或重试耗尽会报告缺口/错误，不能承诺任意故障下无损投递。macOS 激活仅确认系统受理，Windows/Linux adapter 未做原生平台验收。模拟器结果不代表实体手机、蜂窝或公网 TURN 验收。


## SDK 导出复核（2026-11-16）

重新导出后核对目录、引用和 Schema description，公开包仍为 provider-sdk/v2 下的 12 个文件，顶层只有 sdk-manifest.json、README.md、protocol。不包含 Gateway、LAN/RTC、fixtures、Host 源码、Rust 桥接或代码生成器二进制。Notice 共用对象在 common，发布/控制载荷与方向在 provider 注册表中；旧 event.subscribe/unsubscribe/notification 均不再发布。

复核发现 README 仍混有旧插件兼容和原始订阅措辞，通信图也只画 Host 发请求；现已明确纯通知助手的空实例配置、双向管道和 Notice 创建至动作的流程。发布清单原先遗漏最低业务版本，新增从 Provider IDL 派生的 minProtocolVersion；methods/events 与 README 同源更新。新增导出回归覆盖纯通知合同、旧方法缺席、目录白名单和最低版本变化的传播。

本次是说明与发布元信息修正，业务协议仍为 4/4；更新构建暂存和八份独立插件公共合同资源，不修改插件业务代码。审阅包位于 `/Users/wangxin/Documents/CodePet-builds/provider-sdk-review-20261116/provider-sdk/`。当前运行 App 的随包副本是本次修正前已验收的版本，新的导出物由后续 App 打包流程带入；本次未为文档变更中断运行进程。
