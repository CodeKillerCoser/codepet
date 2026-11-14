# 协议生成器的 Windows 路径回归

## 现象与证据

提交 935b096 的桌面 Actions 43617419083 中，macOS App 构建通过，Windows 在 beforeBuildCommand 的 protocol:check 失败，尚未进入 Tauri/Rust App 编译。报错为 `pet-v1.schema.$defs.PetApproval.properties.requestedAt references an unregistered schema: protocol\core\v1\schema.json`。

## 根因与修复

generate.mjs 用 `schemaPath.replace('/v2/', '/v1/')` 建立历史 DTO 引用到活动 Schema 的兼容别名；Windows 原生路径使用反斜杠，替换没有发生。compatibility.mjs 还把本机绝对路径直接拼成 file:// URL 再使用 pathname，可能丢失盘符或错误保留空格/Unicode 转义。

两处统一使用 schema-paths.mjs：本地相对 URI 引用解码后按 native path.resolve 解析；版本目录别名用 dirname/basename/join 计算。只有明确 forwardCompatible 的业务包注册历史别名，不改通道自己的 v1 版本。源码目录版本与服务 version 继续分离，独立 SDK 生成配置没有 sourceVersion 时从实际版本目录识别。

引入历史：上述路径写法已存在于 935b096；首次引入的独立提交未继续追溯。

## 复现与回归防线

运行 npm run protocol:check 和 npm run providers:test。compatibility.test.mjs 分别以 win32/posix 验证盘符、中文空格目录、URI 空格转义及旧 DTO 别名；另复制完整 protocol 到含空格与中文的临时 checkout，实际加载并构建校验图。

Provider protocol conformance 新增 Windows job，checkout 到 `checkout with spaces`，实际执行协议新鲜度、生成器测试及 SDK 导出/staging 测试。9404c43 的运行 43621537821 中 Windows 与 Ubuntu 均已通过。Windows 完整 App 复建运行 43621541675 也已成功，产出安装包；此次完整构建对应路径修复提交 9404c43。后续业务改动仍需各自验证，不能把这次安装包当成后续源码的产物。

修改路径辅助函数时应同时运行 cp-sdk-gen 测试：工具会构造缺省 sourceVersion 的临时生成配置。不要仅修仓库默认配置，使独立导出又退化。后续任何 Schema 引用解析必须使用同一个本地路径入口，不再手拼 file:// 或针对 `/vN/` 做字符串替换。
