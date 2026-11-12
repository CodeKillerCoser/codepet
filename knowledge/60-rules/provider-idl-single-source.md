# Provider 协议以 IDL 为唯一事实来源

## 规则

Provider 公共协议在 `protocol/v1/provider/` 定义，共享对象在 `protocol/v1/{common,agent-domain}/`；Gateway 在同级独立定义。各功能模块分别维护 Schema、methods/events，入口只登记模块文件。代码、兼容视图、对外合同和协议常量必须由同一输入生成；禁止直接修改生成文件或从下载副本反向维护协议。

## 适用场景

新增/调整 Provider 类型、IPC 方法、事件、安装清单、profile、传输常量或生命周期策略，以及 SDK 构建分发。

## 反例

在 methods.json 新增方法，却另手写一份 envelope 方法名；在 README 和 Rust 中分别写心跳周期；只在已导出的 SDK 补字段；Host 手写一套允许值而发布 Schema 另有一套。

## 推荐做法

先修改权威 Schema/注册表/传输描述，运行 `npm run protocol:generate` 更新兼容视图、语言代码与内部协议常量。字段解释放在 Schema description，交互和使用说明放在服务 README；涉及数值的协议说明从 IDL 渲染。安装清单不生成外部插件业务代码，但 Host 解析结构和验证遵循同一 Schema。

`npm run sdk:export -- <空目录>` 与 App staging 使用同一个发布器，产物为 `<目录>/v1/`，携带源摘要。methods/events 和 profile ID 不在多个权威输入重复列举；发布联合类型及安装 transport 枚举由注册表派生。手写机制只能实现合同，不能自行改变协议值。编译器不支持的约束明确报错，不得忽略。

## 来源

[统一生成与分发](../10-architecture/provider-idl-generation-and-distribution.md)。用户已确认 SDK 整包按版本分目录、安装清单和通信生成输入分别放在 protocol/manifest.schema.json 与 protocol/sdk/。

## 验证方式

执行 `npm run protocol:check`、`npm run providers:test`、SDK/Rust Host 回归及独立作者黑盒互通。IDL 测试需覆盖一次输入修改传递到运行时、发布物和说明，缺失 description/ref/default/不支持约束拒绝，以及产出确定性。新代码不得把兼容视图重新作为仓库默认生成源。
