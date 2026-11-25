# 本地模型环境与模型配置

## 背景

此前模型设置仅提供两组原始输入框，缺少已安装环境检测、安装引导和模型列表，未实现用户确定的两块布局。此次将本地环境生命周期和模型连接配置分别实现，保留已有 Model Gateway、Cognee sidecar 与用量统计边界。

## 目标

- “本地运行环境”优先发现系统已有 MLX；缺失时提供安装确认、真实进度、取消和失败重试。
- “模型配置”分别配置语言模型和向量模型；提供供应商预设、可编辑 API 地址、密钥、模型自动检测及手动输入。
- 沿用全局弹窗、灰色按钮交互 token 和字体 token，支持键盘及窄窗口。

## 非目标

不内置模型权重，不自动购买云端 API，不自动启动推理或下载模型；不使用 Python/pip 安装 MLX。环境安装与模型配置保存相互独立。

## 现状理解

模型配置仍位于活动数据目录 `model-gateway/config.json`，不进入 AppSettings 或设置广播。语言模型负责摘要和抽取，向量模型负责 embedding；经 Host 转发的推理继续统计实际用量。

本机实测已有 `/Applications/MLX Core.app`，bundle ID 为 `com.dalcu.mlx-core`，版本 26.9.2。当前同项目官方安装包名称为 MLX-Serve，应用标识保持兼容。本轮验证的官方 v26.11.3 DMG 为 122,812,556 字节，SHA-256 为 `8609126ac89cc1d03cf5715692b7aed1add7cf8dbc928f9c793b610c2e67a98c`，签名 TeamIdentifier 为 `S77TW432YG`。平台要求以 [官方项目](https://github.com/ddalcu/mlx-serve) 为依据：Apple Silicon、macOS 26.2+；本地 API 为 `http://127.0.0.1:11234/v1`。

## 实现路径

### 本地环境

Host 定义统一 `status / prepare / install / cancel / open` 接口，macOS 负责发现和原生安装，其他平台返回能力说明。业务 UI 不执行系统命令。

发现依次检查系统与用户 Applications，再使用 Spotlight 搜索已知 bundle ID，最后检查 CodePet 管理目录。已有系统应用优先，既不覆盖也不升级。主窗口点击“打开 MLX”才启动已验证的应用；“使用本地 API”只修改表单草稿，仍需用户选择模型并保存。

`prepare` 仅读取 [官方发布元数据](https://github.com/ddalcu/mlx-serve/releases)，生成有效期 15 分钟的内存确认 ID；用户确认后提交该 ID 才能安装，不接受前端传入任意安装 URL。安装目录固定为 `PathManager.workspace/tools/local-models`，与活动任务数据目录独立。

安装采用文件锁防止多个进程同时写入，按实际字节显示下载进度，校验官方 SHA-256，再只读挂载 DMG、验证 bundle ID 与发布者签名、复制到临时目录，最后原子发布到 `MLX-Serve.app`。全程不需要管理员权限，不覆盖既有应用，不绕过 Gatekeeper。最终发布阶段短暂禁用取消。

取消通过安装 ID 校验并通知后台任务，终止且等待原生进程退出，再卸载镜像和释放本次临时目录。只有清理流程结束才进入终态。页面离开不取消后台安装，再次进入继续读取状态；进程重启将未完成状态标记为中断，用户可重新安装，不续传下载。异常退出留下的临时目录目前不自动清扫。

### 模型配置与检测

预设本地 MLX、OpenAI、DeepSeek、阿里云百炼、硅基流动及自定义 OpenAI 兼容服务；DeepSeek 仅列在语言模型预设中。API 地址始终可编辑，特定区域、workspace 或代理服务可使用自定义地址。预设地址参考 [OpenAI 模型目录](https://developers.openai.com/api/reference/resources/models/methods/list)、[DeepSeek API](https://api-docs.deepseek.com/)、[百炼 base URL](https://help.aliyun.com/zh/model-studio/base-url) 和 [硅基流动模型列表](https://api-docs.siliconflow.cn/docs/api/models-get)。

模型列表由 Host 直接请求 API base 下的 `/models`，显式 `no_proxy`，不跟随重定向。仅允许 HTTPS 或 loopback HTTP，拒绝 URL 中的账号、查询参数和片段。已保存密钥只在同用途、同 endpoint 时复用；切换地址清除草稿密钥。上游错误正文不返回前端，避免错误响应回显密钥。

有本地地址或可用密钥时自动检测，也可主动检测；不支持列表时保留原配置并提供手动模型名。旧请求与卸载后返回的响应被丢弃。按服务实际提供的类型/能力区分语言、向量及媒体模型；没有类型信息的列表不臆断能力。embedding 维度只读取明确的 `dimensions / embedding_dimensions`，不把模型 hidden_size 当输出维度；未知时展开高级设置要求填写。更换模型清除旧维度。

高级设置保留 plainJson 与既有三个受支持的附加参数；用量折叠显示，刷新用量不覆盖未保存表单。默认仍不开启任务加工。

## 涉及模块

- `frontend/lib/ModelSettings.svelte`：两块装配、配置草稿和保存、用量展示。
- `LocalModelEnvironment.svelte`、`localModelRuntime.ts`：环境状态、全局确认弹窗和进度轮询。
- `ModelProfileEditor.svelte`、`modelProviders.ts`：两种模型共享表单、预设和检测竞态处理；`modelSettings.css` 沿用主题样式。
- `crates/codepet-host/src/local_models/`：统一接口、安装状态、文件锁、下载和平台实现。
- `crates/codepet-model-gateway/src/catalog.rs`：安全复用密钥并检测模型，不计作推理 Token。
- `src-tauri/src/task_ledger.rs`：沿用 main 窗口鉴权，新增环境与模型目录 IPC；不暴露 Agent 安装 MCP。
- `webcontent-contract.json`：新接口要求 backend API 40，需重建原生 App；仅替换 WebContent 无法给旧 Host 增加接口。

## 风险

- 官方安装发布格式、签名或最低系统版本变化：固定源、digest、bundle ID 和签名校验；不满足即显示错误；真实包测试可重跑。
- 安装中断或重复请求：文件锁、确认 ID、取消 ID 和原子发布；单测覆盖并行写入拒绝、取消及中断恢复。本进程状态不提供另一实例的进度镜像。
- 密钥发送到错误地址：不同 endpoint 不复用、禁止重定向、显式 no-proxy；HTTP 测试验证鉴权与错误脱敏。
- 服务能力与维度描述缺失：支持手动配置，不额外调用推理猜测维度；实际本机模型列表测试覆盖媒体分类。
- UI 异步响应和取消后焦点丢失：组件测试覆盖竞态和恢复；浏览器检查全局弹窗、Escape、取消后焦点及 960/720/560px 布局、深浅主题。

## 测试计划

已执行本轮验证：前端全量 282 项测试通过、`npm run check:frontend`、`npm run build`、`npm run check:source-size`，以及 Gateway 测试、Host local_models 单测和 Tauri cargo check。Gateway 的 7 项常规测试、Host 本地环境的 7 项常规测试均通过；3 项按需真实验证另外显式执行并通过。对应终端日志位于 `/tmp/codepet-model-*.log`。

真实外部验证使用明确忽略、按需执行的测试，均不修改生产配置、不启动用户应用：

```sh
cargo test --manifest-path crates/codepet-model-gateway/Cargo.toml installed_mlx_catalog -- --ignored
cargo test --manifest-path crates/codepet-host/Cargo.toml --lib official_latest_release -- --ignored
CODEPET_MLX_QA_DMG=/absolute/path/MLX-Serve.dmg cargo test --manifest-path crates/codepet-host/Cargo.toml --lib signed_package_installs -- --ignored
```

前三项真实验证已通过：本机 MLX 列表只读检测、GitHub 官方 metadata 解析，以及隔离目录内实际签名校验/安装/卸载/拒绝覆盖。UI 通过 `frontend/qa/main-window.html?mlxMissing` 模拟缺失和安装状态；该 fixture 的进度用于交互验收，真实原生安装能力由上述测试单独验证。截图在忽略目录 `artifacts/mlx-runtime-qa/`。

## 知识沉淀

设置模型与 sidecar runbook 指向本文，安装、配置和模型推理职责分别维护在各模块；不新增中心化知识索引。

## 未知项

未使用真实云端密钥测试各供应商的区域/账号权限，也未在 Intel Mac、Windows 真机上执行本次 UI。安装后的首次打开及系统 Gatekeeper 提示由用户在实际 App 中完成；本轮不替换其已安装 MLX。
