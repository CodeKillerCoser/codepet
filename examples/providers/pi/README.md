# Pi 自定义 Provider 验证包

> 历史实验：已由仅凭导出合同开发的 [独立插件示例](../independent/README.md) 替代。当前 App SDK 不再附带这里使用的 Rust 生成器；以下为旧桥接验证记录，不作为当前接入步骤。

这是独立的 TypeScript Provider 示例，用于检验第三方开发体验。它通过现有“添加自定义 Provider”导入，未注册为内置 Provider。Pi 使用官方 [`@earendil-works/pi-coding-agent`](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/sdk.md) SDK，锁定版本 `1.5.2`。

当前 App SDK 只提供 Rust 服务端。因此本示例包含一个实验性 Rust bridge：Host 一侧调用 App SDK 的 mux runtime；Node 一侧使用包内部的 LF JSONL 请求、响应和事件。这是同一 Provider 包的实现细节，不是新增 Host transport，也不是已经发布的 TS SDK。

后续方向已纠正：Rust bridge 只保留为本次联调记录，不作为 TS SDK 的目标结构。下一轮应由 TS SDK 自己实现现有 Provider wire/runtime，Provider 打包为可直接启动的产物，由 Host 按 manifest 的 executable/args/env 加载。DTO/interface 继续从 JSON Schema 自动生成，方法/事件映射从协议注册表生成；补齐分发入口，不要求 Provider 作者手写。当前桥接版尚未完成这个独立 TS 验收标准。

## 构建

需要本机 Node 24+、npm、Rust/Cargo 和已安装 App 的 SDK。已验证 macOS ARM64、Node 26.9.0。默认读取 `/Applications/Code Pet.app/Contents/Resources/provider-sdk`；其他位置用 `CODEPET_SDK_DIR` 指定。

在仓库根目录运行：

```sh
npm ci --prefix examples/providers/pi --bin-links=false
node examples/providers/pi/build.mjs /tmp/codepet-pi-validation/pi-provider
npm run check --prefix examples/providers/pi
npm test --prefix examples/providers/pi
```

bridge 的 Rust SDK 从 **已安装 App** 的 `cp-sdk-gen` 导出，构建时不引用内置 Provider 或 Host 源码。DTO 目前需要从仓库的 canonical TS 生成物复制，这是本实验确认的分发缺口。`types/`、`.build/` 和 `node_modules/` 不提交。

产出目录内含 `codepet-provider.json`、bridge 可执行文件、TS 业务与 worker runtime、三份生成类型及生产依赖。`npm ci --omit=dev --bin-links=false` 避免 `.bin` 符号链接被导入器拒绝；构建末尾递归校验所有文件均为普通文件/目录。当前包约 161 MB，尚未优化体积。

产物绑定构建机器的 Node 路径和平台依赖，供本机验证；跨机器分发需要单独解决 Node 运行时与平台包，不是一个通用安装包。

## 配置和导入

1. 使用 pi 自己的登录、环境变量或 `~/.pi/agent/models.json` / `auth.json` 配置模型。密钥不写入 Provider manifest。
2. 若使用其他 pi 配置目录，在**首次导入前**给产出 manifest 的 `env` 增加 `CODEPET_PI_AGENT_DIR`，值为该目录的绝对路径。
3. 在 Code Pet 的运行时设置中点击“添加自定义 Provider”，选择产出的 `codepet-provider.json`。显示名称为 `Pi (TypeScript)`，插件 ID 为 `dev.codepet.example.pi`，默认实例为 `pi-example`。
4. 从现有 Gateway 客户端选择该实例，创建工作目录下的 `read-only` 会话，再发文本请求。这不会给桌面 App 新增完整聊天页面。

只声明已实现的能力：独立工作目录会话、列表、按 turn 分页的历史、交互接管/释放、发送、中断。输出 text delta、完整消息、工具 item 和 turn/conversation 状态。`read/grep/find/ls` 为本次工具范围，关闭自动扩展、skills 和 prompt templates。未提供项目管理、审批、Observation、用量统计、未读聚合和写入工具。

私有 SQLite 路径完全来自 `provider.initialize.directories.databasePath`，仅新增 `pi_conversations` 表；pi 原生 session 文件放在该 Provider 的 data/sessions。不会读取 Host 共享数据库环境变量。stderr 由 Host 落日志。插件重启后保留历史和 pi session 索引；运行中异常退出的 turn 恢复为失败，不能自动重发已接收输入。

导入器拒绝同 ID 覆盖。示例代码迭代请使用测试的临时安装目录，或按既有插件维护流程处理已安装包，不直接覆盖运行中的用户会话。

## 真实子进程验收

构建完成后运行：

```sh
node examples/providers/pi/tests/host-validation.ts /tmp/codepet-pi-validation/pi-provider
```

测试启动本机确定性 OpenAI-compatible SSE 服务，配置隔离的 pi models.json，复制包到临时目录，再运行 Host 的 opt-in 测试。它使用生产导入路径、真实 mux bootstrap、Initialize、Gateway、本机 Node、真实 pi SDK/工具和 SQLite；模型响应由 fixture 控制，**没有调用真实云端模型**。标准 `cargo test` 不会自动执行该需要外部包的用例；脚本显式传 `--ignored`。

验证范围：导入、Gateway 发现、create/acquire/send、实际读取 proof.txt 的 pi 工具循环、流式与 item 的稳定 ID、重复请求、中断、私有库、stderr 日志、restart 历史和重复导入拒绝。直接 TS 测试补充路由隔离、请求 ID 冲突、worker 并发关联、UTF-8/U+2028、截断帧拒绝。

使用用户明确指定的 DeepSeek Flash 做云端验收（会实际请求付费模型，不在默认测试中运行）：

```sh
node examples/providers/pi/tests/live-validation.ts /tmp/codepet-pi-validation/pi-provider
```

该脚本要求 `.zshrc` 中配置 `DS_API_KEY`，使用独立临时工作目录与模型配置，仅发送生成的 proof.txt 测试内容。模型 ID 按 [DeepSeek 官方接口](https://api-docs.deepseek.com/en/) 使用 `deepseek-flash`。已实际验证读取工具、真实流式回复及重启后的历史；中断等可控时序仍由本地 fixture 验证。

本机已在 `~/.pi/agent/models.json` 创建 `codepet-deepseek/deepseek-flash` 配置，认证在请求时通过 zsh 读取现有变量，文件中没有明文密钥。Code Pet 已正常重启并确认导入插件仍在线。单独导入后的模块实例也能识别该模型。该配置是本机验收环境，不作为仓库或安装包的一部分。

当前结果和 SDK 设计判断见 [验收记录](../../../knowledge/40-runbooks/pi-external-provider-sdk-validation.md)。原生 App 的自定义入口导入已通过，显示在线与 Harness 可用；真实 DeepSeek Flash 的 Host/Gateway 验收也已通过；尚未验证手机/TUI 客户端页面。
