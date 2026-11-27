# 设备连接、业务加载与 Provider 故障隔离

## 规则

通道认证、Gateway 握手与事件订阅成功后设备即在线。Provider 能力探测、项目/最近/历史读取和任务执行各自拥有状态与错误；任一 Provider 的无效数据、业务错误、超时或退出不得使整个设备离线，也不得阻塞其他 Provider。

## 适用场景

Remote 的 DeviceSession、Provider 描述与列表初始加载、分页、Provider 切换/重启/卸载重装，以及 Host Gateway/PluginManager 的跨插件调度。真实断链、认证失效、握手不兼容和全局事件流损坏仍属于连接失败，不能一律吞掉。

## 反例

在 `_connect` 中等待全部 describe，再串行加载选中 Provider 的项目和对话；项目校验错误进入统一 catch 后关闭 socket、清空握手并把设备设为 failed。另一个反例是共用分页 loading/error 门，使 A 的慢分页挡住用户切换后 B 的请求。

## 推荐做法

连接先公布 online 并安装状态监听；业务读取独立调度。按 Provider 和业务域保存 loading/ready/failed、保留已有数据并提供局部重试。能力尚未得到不能显示“不支持”。只为选中项加载列表，描述可并发且逐项独立完成；慢项不构成其他项的完成屏障。

读取设置有限预算；写入不因恢复自动重发。请求同时绑定设备 runtime generation、Provider epoch 和查询范围；重启、能力变化、离线及卸载重装使旧请求失效。旧 finally 不能清除新门，错误不跨 tab。独立请求的快照/事件仍需游标围栏，不能为了后台加载丢失事件或让旧快照覆盖新事件。

## 来源

2026-11-27 实体手机日志中 project.list 远端错误传播至 DeviceSession._connect，用户要求拆分连接与业务并加强 Provider 隔离。证据和实现见 [排查与修复](../40-runbooks/remote-provider-business-failure-isolation.md)。

## 验证方式

注入描述挂起、无效项目、业务错误、请求超时、进程退出、跨 tab 慢分页、旧 generation 与同代卸载重装回包；确认设备在线、健康 Provider 可查询与发送、局部错误可重试。另注入真实断链/认证失败，确认仍失败/重连。覆盖快照期间事件、窄屏布局、键盘焦点/重试、懒加载及缓存分页。
