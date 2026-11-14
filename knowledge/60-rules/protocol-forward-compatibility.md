# Provider 与 Gateway 的前向兼容约束

## 规则

当前业务范围使用 protocol.version/minVersion，响应另带 selectedVersion；接收未知内容不能提升为执行能力或授权，已知数据损坏不能降格为 Unknown。

## 适用场景

两层 IDL、语言生成器、Host/Provider 生命周期及 UI 消费。配对、stdio profile、SDK 发布版本分别演进。

## 反例

只放宽 serde/Dart 的字段检查而不处理枚举；把任何联合解码错误吞掉；把未知状态当 idle/completed；未知事件无限重放；请求编码把保存的未来授权值原样发回；只校验导出相对文件存在，忽略 `$id` 的 URI 解析。

## 推荐做法

用规范化 IDL 同时派生已知结构校验图与类型，接收区分已知和未知判别分支，发送检查认识且正向声明支持的选择。客户端复核最高共同版本，Provider descriptor 与 Initialize 区间一致。无交集即拒绝，不自动回退或重放写入。异步探测使用明确 pending 状态，不能以空版本伪造已验证安装。

未知 Gateway 事件先校验 JSON-RPC 信封及非空游标，再跳过并推进游标；已知非法事件继续报错。未知能力不调用，未知状态不能自动派发。公共 Schema 的 `$id` 必须与发布版本和相对路径一致，以标准校验器从独立导出包验证。

## 来源

`knowledge/50-decisions/provider-gateway-forward-compatible-versioning.md` 及 2026-11-14 独立 Provider/两端验收。

## 验证方式

协议生成、公开导出/Ajv、Rust/TS/Dart codec，以及跨 Provider 并发集成。覆盖非法范围/选择、未知字段/枚举/联合/事件、合法游标推进、已知损坏拒绝、请求选择严格和无写重放；两端构建后运行隔离 QA App 与 Android 原生测试。忽略用例与真实验收边界分别记录。
