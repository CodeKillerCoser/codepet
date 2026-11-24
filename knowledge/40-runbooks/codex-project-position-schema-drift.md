# Codex 项目响应移除 position 导致手机项目与发送能力丢失

## 现象

2026-11-25，手机已连接 Code Pet，但 Codex 项目列表不可用，进入旧会话或新建会话后显示“当前 Provider 不支持继续对话”。本机安装版为 0.3.11-beta，实际 Harness 是 ChatGPT 内置 `codex-cli 0.171.0-alpha.13`。

## 证据

- 本机 `logs/events.jsonl` 的 2026-11-25 01:49–02:54（北京时间）多次 ready 快照均有 conversation.list/get/create，但缺少 project.list、turn.start 和 turnSend。Harness 已启动、认证可读，不能把插件在线等同于业务能力完整。
- 对同一原生 binary 执行 initialize、model/list、project/list：模型目录正常，非空项目响应包含 id/name/roots/metadata/createdAt/updatedAt/recencyAt，不含 position。仅记录字段类型与数量，未记录项目正文、路径或认证内容。
- `app-server generate-json-schema --experimental --out /tmp/codepet-codex-0171-schema` 的 ProjectListResponse 中 Project 不再定义 position；ProjectListParams 明确默认按 position 排序，但数值位置不对外暴露，调用方应使用返回顺序。
- 原 `protocol.rs::CodexProject.position` 是必填 i64；`provider.rs::refresh_metadata` 把模型和项目探测合并，项目解码失败时直接丢弃整个 catalog。错误经 `.ok()`/返回 None 隐藏。
- Host `gateway.rs::gateway_capabilities` 将 turn.start 映射为 turn.send；Remote `conversation_detail_controller.dart::composerHint` 在缺少 turn.send 时显示上述提示。两个现象可由同一个 Provider 发现失败解释，无需假设手机链路断开。
- 新增无 position fixture 后，修复前测试 `projects_without_native_positions_preserve_order_across_pages_and_get` 稳定因等待后台 capabilities 超时失败。

## 根因

当前 Codex 原生项目响应与旧 DTO 不兼容，触发项目解码失败；Code Pet 将项目探测与发送能力发现耦合，进一步移除了本来可用的聊天能力。这是上游契约变化暴露出的适配与故障隔离缺口。不能表述为公共 Provider/Gateway 协议版本升级失败。

## 引入历史

`git blame` 确认必填 position 来自 `cb703d869`（2026-09-03）；catalog 合并及项目异常时返回 None 来自 `1c4e33df2` / `ad132932e`（2026-09-07）。近期 recent/Notice 改动没有修改这些分支。现已取得官方 0.166.0-alpha.5.1 binary 并校验 npm SHA512：其 schema 仍要求 position；0.171.0-alpha.13 则已移除。首次移除的具体版本和提交未确认，上界按用户指定的上次适配版本记录。

## 修复方案

按四个变化接口独立分派：`project/list`、`read`、`create`、`update` 各自判断实际运行进程版本，`<= 0.166.0-alpha.5.1` 使用原严格 position:i64 DTO；最后 else 使用 `CompatibleV0171Alpha13`，默认服务更高版本。完整 SemVer 比较保留预发布数字段、忽略 build metadata 对优先级的影响，无法识别版本明确诊断。未变化接口不加分支，mapper.rs 与公共协议不变。

新版专用 Project DTO 在兼容模块内解码，按官方列表顺序生成 Provider-defined rank，不以 ID/recencyAt 代替。首屏直接按数组排名；空续页直接返回；后续页及 get/create/update 用原生项目目录解析全局排名，保持 cursor/limit 透传。扫描每页 100、最多 100 页，共用 30 秒 deadline；重复 cursor 或目标消失时明确失败。写入不换分支重试。

项目探测异常只关闭项目能力，保留成功发现的模型与 turn.start；记录 method 和标准错误 code，不打印业务 payload。仍需项目探测成功才能声明项目操作，不把失败当成功或重放写入。

## 涉及模块

- Codex `protocol.rs`：旧 DTO 保持严格整数，补旧契约测试；`client/compatible/`：新 DTO、局部归一化和完整版本比较。`mapper.rs` 保持原实现。
- `client/projects.rs`：收拢原生项目操作，逐接口选择兼容方法，保持旧 API 行为和原生分页游标。
- `provider.rs`：隔离项目探测故障对发送能力的影响，保留生命周期 epoch 和单次完整 ready 发布。
- Provider fixture、vertical 与 native_runtime 测试：覆盖新旧契约、跨页排序、get/create/update、损坏数据和真实 CLI。Host、Remote、公共 IDL/SDK 无修改。

## 官方契约对照（2026-11-25）

- 0.166.0-alpha.5.1 与 0.171.0-alpha.13 实验 schema 经 `scripts/diff_codex_schema.py` 比较。四个项目响应只发生 Project.position 属性及 required 列表变化；请求参数、delete、ModelListResponse、ThreadStartParams 不变。Thread/Turn 响应的新增可选字段与当前宽容读取兼容，未为其新建分支。模型筛选新增可选 modelProvider；审批 humanCommand 依赖新 opt-in，本客户端未声明该能力。
- 项目 schema 快照位于 `tests/fixtures/contracts/`；真实导出指纹分别为 `8c7e57e6ba221271a1cf55db2df2da52da6eaafd33a5c7b1a44dbd49136f67e4`、`5afa92765b4717c360cf6e9e08b292f5c91e3f7e4b3241b899fb25460e166f4c`。完整临时导出不作为生产依赖。

## 验证结果

- `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex`：首次通过 86 单元测试和 73 纵向测试；新增边界用例后并行全量中，原有 saturated-stop 测试出现一次 mux command receiver closed，单独复跑通过。最终 `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --quiet -- --test-threads=4`：86 单元 + 75 纵向测试通过，4 项外部环境测试默认忽略（其中新增真实探针已显式单独执行通过）。
- `CODE_PET_CODEX_BIN='/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex' cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --test native_runtime installed_codex_projects_and_chat_capabilities -- --ignored --nocapture`：通过。使用真实用户目录，非空项目首屏、get、第二页排名以及 turn.start/turnSend 能力均验证成功；未发送模型请求、未 resume、未创建或修改用户项目。
- 一次早期隔离 CODEX_HOME 探针项目为空，只能证明启动与模型发现，未将其作为新项目字段验收证据；上述真实非空目录测试补齐该缺口。
- 后续逐接口实现全量：88 单元 + 76 纵向通过；4 项外部环境测试默认忽略，真实新版项目探针另行显式通过。Host `manager_gateway` 58 项 + `builtin_provider_integration` 5 项通过；`npm run protocol:check` 41 项、`npm run providers:test` 6 项通过，公共协议和生成 SDK 无变更。
- 本机 Android API36 `CodePet_Protocol_QA_20261112`、Remote 0.1.2 正常配对安装版 Host。`before-conversation.png` 复现“当前 Provider 不支持继续对话”。热更新后四个原生项目可列出，新建对话恢复项目、工作区、权限、模型选择。
- 仅替换 `/Applications/Code Pet.app` 内 Codex Provider 并重新签名校验，随后终止旧 Provider，访问历史由现有恢复机制触发该插件独立重载。Host PID 14417 保持，Provider 14425→15994，最终构建再独立重载为 26779；没有重启 Host。备份 `/Users/wangxin/Documents/CodePet-backups/codex-interface-compat-20261125/`。Remote 重连后重新获取完整目录；不能把强制结束进程当作新增在线升级产品流程。
- Android 新建 Main/read-only 专用测试会话，首轮完成并显示模型回复；最终 Provider 重载及 Remote 重连后，同一会话第二轮返回 COMPATOK。原生 App read_thread 同时确认两轮 completed，第二轮无工具调用。最终跨端归属结果见下节。`artifacts/codex-interface-compat-20261125/` 留存 schema 差异、安装指纹和前后截图。

## 回归防线

保留无 position 且 ID 顺序与列表顺序相反的两页 fixture，确保手机按 position 排序时不会重排；覆盖项目探测 RPC 错误、已知字段损坏但聊天能力保留，及排名扫描重复 cursor/目标消失。真实 native 测试显式要求至少一个已有项目，防止空目录绕过字段解码。适配规则补充到 [Provider 原生 API 兼容边界](../60-rules/provider-native-api-compatibility.md)。

## 规约候选

已更新原生 API 兼容规约：可独立探测的能力域应隔离失败；原生排序字段移除后根据正式契约转换，不能修改公共协议或伪造排序值。

## 未知项

上游变更首次发布版本；Windows 安装环境与实体手机的完整发送/审批流程。Android 模拟器不替代这些平台证据。项目并发移动/删除期间，排名扫描与业务响应无共同快照；额外只读扫描有性能成本。通过有界失败、重复游标/目标消失测试防止假排名和无限查询；写入成功后排名读取失败也不自动重试 create/update，应刷新目录核对。


## 跨端归属验收发现的独立限制（2026-11-25）

实际 QA 会话 `01a20f55-bfb5-7cf0-b97a-0574644b0bfd` 从 Remote 的 codepet 项目创建。SQLite 只读核对：`threads.project_id=01a0e3f2-48c1-7ee1-8aa6-f1c121dec945`，指向既有原生 codepet 项目；执行目录为项目主目录。原生 App 的 `list_threads` 却返回同 ID 的 `projectId=null`，并将其 itemKey 放在 `chats` section，其侧栏 codepet 使用旧 ID `886f7844-5558-4b76-a0c9-903515d2b537`。本地迁移检查点仍有 `threadAssignmentsMigrated=false`，新会话没有旧 `thread-project-assignments`。

这与既有跨端归属排查记录的迁移机制一致，但不能仅凭检查点推断运行时 feature flag 的取值。官方 app-server `project/import`、`thread/metadata/update`、`thread/start.projectId` 管理原生归属，schema 没有提供桌面旧映射写入口；再写同一个 native projectId 不能证明桌面会采用。此次保留原生身份，没有写桌面私有 JSON、改开关、调用强制接管或把 cwd 当项目。**因此“原生 App 自动归入原项目”验收未通过，不能将本次兼容修复标记为完整跨端通过。**

独立重载期间 Remote 出现过 `Recent event providerId does not belong to the connected Host`，重新连接后重建目录；之前还出现 recent snapshot cursor 过期。它们属于 Host/Remote 清单与 recent 生命周期，不能计作 Codex schema 适配成功或据此重启整个 Host。本次没有扩改这些模块。已有桌面会话继续时的 active writer 冲突也保持原生错误，没有执行会结束其他 Codex 进程的强制接管。


### 旧版归属修复历史核对（2026-11-25）

用户指出旧版曾修复归属后，核对 Git 全部现存引用及相关 diff：

| 提交 | 当时解决的行为 |
| --- | --- |
| `08de3009`（2026-08-31） | Code Pet 读取会话时按 Git/common-dir 归并 worktree 的展示项目，解决 Remote 重复项目；未向原生 App 写归属 |
| `f4fdf5f8`（2026-09-04） | 补齐 Provider 会话列表的项目归属，原生 projectId 缺失时曾按 cwd/project roots 推断 |
| `be531f60`（2026-09-04） | 用 Desktop 旧关联与 legacy→native ID 映射替代上述路径推断，补齐 Provider 读取及筛选，覆盖 worktree |

三项提交均为当前 HEAD 的祖先。旧关联兼容仍由 `provider/directory_list.rs` 与 `provider/ordered_ids.rs` 调用，未被本次项目响应适配删除。复跑 `cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --lib project_assignment_compat_tests --quiet`（2 项）及 vertical 的 `project_owned_conversation_uses_native_project_id_without_inferring_cwd -- --exact`（1 项），均通过。这些提交和测试验证读取补齐、筛选与 native ID 传递，不验证 Remote 新建后原生 App 自动采用旧侧栏关联。

11 月 8 日已有排查也记录了“native project_id 正确、旧 assignment 缺失”的相同数据状态，早于这次 0.171 适配。不能把用户记得的修复概括为从未存在；同样不能在没有旧版完整跨端通过证据时，断言现在是旧修复回归或已经确定官方开关是根因。


## 本次主干提交范围（2026-11-25）

用户确认“Code Pet 新建 → 原生 App 自动归组”为既有问题，本次先不处理，不作为此次兼容修复提交的阻塞项。历史验收失败证据保留，不将其改记为通过；原生项目 ID、显式归属参数及现有旧关联读取兼容继续保留。

本次提交包含项目接口逐版本兼容、项目探测失败保留模型/发送能力、新旧契约与分页回归、schema 差异开发工具，以及将导出的 Provider SDK README 改为 SKILL.md 并写入逐接口兼容最佳实践。截图、完整 schema 导出和 SDK ZIP 为本机验收产物，不纳入源码提交。与本次无关的工作区修改保留。
