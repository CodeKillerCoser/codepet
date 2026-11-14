# 协议代码生成

当前权威源是 `protocol/v2/`。`protocol/codegen.json.sourceVersion` 选择源目录，各层 `protocol.json` 自己维护业务 version/minVersion。旧 v1 作为冻结基线，可由测试显式加载，不被默认生成覆盖。功能模块中的 Schema 与 methods/events 是唯一 DTO 和注册表来源。

`modular-contract.mjs` 校验引用闭包、描述、依赖方向及方法重名，组装各层 Schema/manifest 视图和公开 Provider SDK。`generate.mjs` 构建规范化 IR，再由 Rust、TypeScript、Dart target 输出。内部包标识 `core-v1` 等是既有生成器键，不作为当前业务版本；对外常量来自服务元数据。`codepet.protocol.codegen/v1` 是生成器插件接口版本，也不是业务协议版本。Python target 尚未实现，选择时会明确失败。

Rust 业务对象忽略未知字段，枚举生成 `Unrecognized(String)`，联合生成 `Unrecognized(Value)`；已知判别分支解码失败不得退入未知分支。`compatibility.mjs` 从同一 IDL 派生校验图，生成请求/响应与事件校验入口；Rust 公共执行器严格检查已知结构、范围和格式，请求中的未知选择返回非法参数。TypeScript 提供实际的 `decodeProtocolValue`、`validateProtocolValue` 和 `encodeProtocolValue`，接收原值保留，发送保持严格。接口声明仍描述本版认识的结构；调用者必须显式识别未知接收数据，不能强制断言为已支持类型。

Dart 从 IR 生成值类枚举、未知联合容器、约束 codec 和 typed client。未知事件名称保留原值，校验基础信封和游标；应用仅跳过已确认未知的内容，继续推进游标。请求编码按同一校验图禁止发送未知选择。共用的区间协商函数放在 core SDK，不复制握手 DTO。

公开 SDK 只发布 IDL 和中文说明，不携带 Host、Gateway、fixtures 或语言工具。Schema enum/oneOf 定义本版已知形状，接收 codec 的前向容错由兼容政策和生成规则共同执行；普通 JSON Schema 校验器不会自动产生 Unknown 容器。传输 profile、握手和资源限制仍独立严格校验，不因业务对象开放而放宽。

`cp-sdk-gen` 与仓库生成共用加载器、IR、语言 target 和运行时脚手架；`sync_remote_sdk.mjs` 将 Gateway、旧包名的配对 SDK、通道控制 SDK 同步到手机仓库，禁止手改生成物。源摘要用于检查同源，不替代业务区间协商。

运行 `npm run protocol:generate` 更新生成物，`npm run protocol:check` 验证新鲜度、冻结基线、结构与 TS codec；`npm run providers:test` 验证发布闭包；`npm exec --yes --package=bun -- bun test tools/cp-sdk-gen/cp-sdk-gen.test.mjs` 验证独立生成和手机同步。Rust SDK、Host/Provider 集成与两端应用回归需要另行运行，生成器通过不代表联调通过。
