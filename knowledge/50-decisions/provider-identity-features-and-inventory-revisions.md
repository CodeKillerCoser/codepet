# Provider 身份、功能声明与清单修订号

状态：2026-11-15，按用户确认的第 1、2、4 项整理实施。协议输入仍位于 protocol/v2，Provider 与 Gateway 业务范围分别增量更新到 3/2；SDK 目录 v2、安装 Manifest v2、三种 stdio profile v1 保持独立。

## 背景与目标

现有 Gateway 的 providerId 实际是实例身份，而桌面管理桥接同名字段指插件包；UI 通过 conversation/turn 方法前缀推断插件类型，会漏掉没有 Agent 实例的通知插件，也无法表达组合功能。清单原有 changed 事件只支持更新，卸载依赖下一次心跳纠正。并发心跳与旧查询响应可能覆盖刚收到的状态，需要定义统一的新旧关系。

本次目标是明确两种身份、由插件声明功能、补齐实时移除与全量校正，以及修复 Windows 生成器路径问题。Observation 业务载荷类型化、云邀请 IDL 和 transport 兼容输出拆分不在本次范围；不新增第二个插件注册中心，不将本地安装管理开放给远程 Gateway。

## 当前边界与实现

### 身份和功能

pluginId 标识插件包，instanceId 标识 Host 管理的实例。Gateway 保留既有 ProviderSummary.id、路由 providerId 的 wire 名称，并在 Schema 明确其均指实例；摘要新增可选 pluginId，供多个实例归属于同一插件。桌面连接快照明确提供 pluginId 与 instanceId，旧 providerId/id 保留为局部兼容别名。AgentRuntime.providerId 仍是历史本地管理字段，不能拿来调用 Gateway 实例方法。

ProviderPluginDescriptor 新增可选 features，取值 agent、observation、runtimeInventory，可组合。前两项描述会话执行与通知观测职责，后一项声明需要 Host 检测和选择外部运行时。声明由 Initialize 返回，Describe 必须保持一致；安装 Manifest 不再重复维护这份声明。实例 methods 继续决定具体操作是否支持，功能标签不能授予方法权限。

缺失 features 表示旧插件未声明；空数组表示没有声明已知功能。未知名称保留解码但不启用。桌面 UI 从明确声明展示多个友好标签，去掉额外 provider.describe 方法前缀探测。纯通知插件可以没有配置实例；现有 instanceKinds 是支持的种类集合，不代表必须创建实例。新插件未声明 runtimeInventory 时，Host 返回无需选择运行时且不探测该 API，选择请求明确拒绝；旧插件未声明 features 时保留原有查询兼容路径。内置三个 Provider 按实际实现显式声明 agent、observation 与 runtimeInventory。

### Registry 投影与事件

providers/manager/registry.rs 继续负责安装、查询和完整卸载。Gateway 的 provider_inventory.rs 只投影当前活动实例摘要：比较成员及摘要，变化时分配 inventoryRevision，发 provider.changed；成员消失发 provider.removed。插件卸载会移除全部可见实例；仍可见但停止或禁用的实例只更新状态。纯通知插件不伪造远程可操作实例。

握手、provider.list 和 protocol.ping 通过同一投影取得完整清单与修订号。投影 gate 只覆盖内存快照与修订提交，不执行 Provider RPC、文件删除或业务载荷处理；原有按插件有界 worker 和跨 Provider 并行请求保留。Registry 状态和运行时变更主动唤醒投影，不能等下一次 ping 才发送移除。

inventoryEpoch 标识 Gateway 的运行周期，重启后改变；revision 只在相同 epoch 内单调递增。两字段同时提供，旧端同时缺失；进程 generation 与事件 cursor 各自独立。客户端重连以 handshake 快照重新建立 revision 基线，订阅快照边界后事件。事件不高于已应用 revision 时跳过业务但推进 cursor；完整响应低于已应用 revision 时丢弃，相等时可校正集合。全量清单按替换解释，不能只追加。

手机移除实例与能力缓存、停止相关观察并调整选中项，保留已有历史内容。描述查询跨越移除/重装时禁止写回；描述响应仍需满足自身摘要与能力修订一致，不能用旧快照的 capabilityRevision 阻止一次未跨越并发变更的合法刷新。新的修订号可接受重装后较低的 generation，不能用旧进程代次挡住新安装。通道自动恢复/切换时，会话适配器检查候选握手 epoch；不同周期即重建业务快照，不尝试复用恰好合法的旧数字 cursor，也不重放写请求。旧 Host 不带修订号时继续使用原有更新与心跳校正；旧操作端忽略未知 removed 事件，下一次心跳纠正删除结果。

## 涉及模块及验证风险

| 模块 | 原因、风险与验证路径 |
| --- | --- |
| protocol/v2/common、provider/lifecycle、gateway/provider/session | 身份和增量字段是唯一 IDL 输入；生成、标准 Schema 导出及旧 2/2 插件验收防止误提高兼容下界 |
| codegen、发布脚本与 Windows CI | 原生分隔符和 URI pathname 可能造成别名/引用失败；Windows drive 单测、含空格/中文目录加载、Windows 实际生成与导出检查 |
| Host manager 与 Gateway 清单投影 | 卸载、重装、多实例或慢 Provider 不得污染其他实例；Registry 完整卸载、事件主动到达、重放、三 profile 并发及全量 workspace 测试 |
| 桌面 tauri_bridge 与 ProviderRuntimeInfo | 消除类型推断，保留旧管理别名；连接视图、组合功能、无实例通知插件和运行时控件验证，原生查看布局及移除操作 |
| Remote 生成 SDK、mapper、客户端与 DeviceSession | 旧心跳、旧事件及迟到 describe 可能恢复已删除项；专门竞态测试、全量 Flutter 检查和新 App 的 LAN/原生 RTC 联调 |
| 独立 Provider 示例 | 公开资料必须足以实现插件；作者仅获得导出 SDK 与自身源码，源码及四包各 27 项，当前 3/2 与旧 2/2 三 profile 黑盒验证 |

## 验证与知识沉淀

命令及原生联调结果记录到 desktop-android-protocol-joint-test runbook；Windows 故障证据和回归防线见 protocol-codegen-windows-paths。公开 SDK sourceDigest 为 sha256:7736569e62b5bc27137b652fb388ada04c4e3f1327dc8d94103f27394b037a9f。生成一致性只证明来源一致，不能替代运行时联调。

未知项：实体手机、公网 TURN 与性能压力指标不在本轮本机模拟器验收内。路径修复提交的 Windows 完整构建已通过；补充 epoch 后的最终原生界面复验因锁屏待完成，具体状态以联调 runbook 为准。
