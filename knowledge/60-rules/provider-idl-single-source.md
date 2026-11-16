# Provider 协议以 IDL 为唯一事实来源

## 规则

Provider 公共协议在 `protocol/v2/provider/` 定义，共享对象在 `protocol/v2/{common,agent-domain}/`；Gateway 在同级独立定义。各功能模块分别维护 Schema、methods/events，入口只登记模块文件。代码、兼容视图、对外合同和协议常量必须由同一输入生成；禁止直接修改生成文件或从下载副本反向维护协议。

## 适用场景

新增/调整 Provider 类型、IPC 方法、事件、安装清单、profile、传输常量或生命周期策略，以及 SDK 构建分发。

## 反例

在 methods.json 新增方法，却另手写一份 envelope 方法名；在 README 和 Rust 中分别写心跳周期；只在已导出的 SDK 补字段；Host 手写一套允许值而发布 Schema 另有一套。

## 推荐做法

先修改权威 Schema/注册表/传输描述，运行 `npm run protocol:generate` 更新兼容视图、语言代码与内部协议常量。字段解释放在 Schema description，交互和使用说明放在服务 README；涉及数值的协议说明从 IDL 渲染。安装清单不生成外部插件业务代码，但 Host 解析结构和验证遵循同一 Schema。

`npm run sdk:export -- <空目录>` 与 App staging 使用同一个发布器，产物为 `<目录>/v2/`，携带源摘要。methods/events 和 profile ID 不在多个权威输入重复列举；发布联合类型及安装 transport 枚举由注册表派生。手写机制只能实现合同，不能自行改变协议值。语言生成器不支持的类型形状必须明确报错；Manifest 校验关键字原样交给标准 JSON Schema validator，不维护自定义关键词子集。

## 安装清单校验

Host 使用开源 Rust `jsonschema` 库直接校验发布的 Manifest Schema。生成链路只嵌入这份 Schema 和生成 DTO，不生成字段校验代码。发现、加载、导入共用一个缓存的 validator；导入校验发生在复制包、注册和启动之前。校验失败只显示字段 JSON Pointer 与约束名，不回显 Env 等字段值。

Manifest 只接受当前版本，必须包含 icon、command 和 transport；不保留旧测试清单的版本转换、executable 别名、缺省 transport、缺图标或加载豁免。测试清单应跟随当前协议更新，不能以兼容旧 fixture 为由扩展运行时行为。业务 version/minVersion 的前向兼容是另一层协议，不受此安装清单规则影响。

卡片固定展示插件版本、运行时路径、运行时版本和工作目录。类型标签只由 features 决定：agent 显示“智能体”，observation 显示“通知助手”；同时声明时显示两种标签，缺少声明时不发明第三类，也不按 Provider ID 或方法名前缀推断。缺少运行时信息显示“未提供”，纯通知助手的运行时显示“不需要”，不得用插件进程 command/version 代替执行环境。

## 来源

[统一生成与分发](../10-architecture/provider-idl-generation-and-distribution.md)。用户已确认 SDK 整包按版本分目录、安装清单和通信生成输入分别放在 protocol/manifest.schema.json 与 protocol/sdk/。

## 验证方式

执行 `npm run protocol:check`、`npm run providers:test`、SDK/Rust Host 回归及独立作者黑盒互通。IDL 测试需覆盖一次输入修改传递到运行时、发布物和说明，缺失 description/ref/default/不支持约束拒绝，以及产出确定性。新代码不得把兼容视图重新作为仓库默认生成源。

2026-11-15 回归：`ProviderRuntimeInfo.test.ts` 覆盖两个标签及无声明无标签；Registry 测试证明缺 icon 和旧版本在发现/导入均被拒绝；`provider-contract.test.mjs` 验证 Host 嵌入的 Schema 与 SDK 发布物完全一致且透传标准关键词。


## Notice 链路

来源原始 Hook/消息字段只在 Provider 解释，Host 不识别各来源的状态枚举。Notice 通过标准 Provider 反向请求发布，桌宠通过标准 Gateway 本地模块消费；不得恢复 raw event.notification 或私有 Pet Gateway 作为并行链路。finished 与 status 分开，断开不得伪造结束。

每个 Provider 的接收、业务任务和背压独立；共用数据库时，不得持有全局 Provider 锁执行或等待业务。mux 入站 RPC 的响应必须结束子流。生成的校验器必须按 $ref 所属包定位 common 对象；测试应覆盖跨包直接响应。当前 Provider 方法方向仅 hostToPlugin/pluginToHost，历史冻结 IDL 不据此改写。

通知动作执行前先持久化 executionId 回执，未知结果不得重放；原始观察 Hook 不得自行获得回复或授权能力。新增文本输入入口必须在原生桌宠的非激活窗口上验收键盘焦点；只在用户打开编辑器时请求焦点。

SDK 发布元信息必须包含 protocolVersion/minProtocolVersion，均从当前 Provider IDL 的范围派生或校验；README 使用同一模型插值，不手写最低版本。导出回归要修改 IDL 的 minVersion，确认 SDK 清单、methods/events 和使用说明一起更新。目录版本、清单格式与业务区间分开说明，不能通过名称里的 v2 推断业务版本。

协议迁移必须同步原生 IPC 的界面测试 mock。桌宠验收通过标准 `codepet_gateway_request` 的 `notice.list` 和 `notice.changed` 事件提供数据，不能保留已删除的私有 Pet RPC。简写测试场景只允许在 mock 内转换为 Notice；生产摘要与展开卡片都从 `content.title` 读取标题，分组使用完整 pluginId，更新通过递增 revision 的事件传递。执行 `test:pet:ui`、`test:pet:motion` 和 `test:pet:stack`，分别验证两个视图与焦点、窗口展开动画和完成卡片堆叠，避免生成代码正确但视图仍读旧字段。
