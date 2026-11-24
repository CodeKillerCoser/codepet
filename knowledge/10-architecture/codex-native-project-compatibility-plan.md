# Codex 逐接口版本兼容改造方案

## 背景

2026-11-25 用户确定：保留 Codex 原生项目管理与跨端会话归属，Provider/Gateway 协议不变；按接口适配上游，未变化接口不调整。本文取代此前的整实例兼容计划、公共字段扩展和运行时 schema 预检方案。

状态：2026-11-25 已按逐接口结构实现并热替换 Codex Provider，Host 未重启。项目与发送恢复。用户确认暂缓处理既有的“Code Pet 新建 → 原生 App 自动归组”问题，本次提交保留正确的原生项目身份，并以项目列表及对话恢复作为验收范围；不将侧栏归属记为已修复。实际证据见[故障记录](../40-runbooks/codex-project-position-schema-drift.md)。

## 目标

- 哪几个官方接口变化，就只调整这些接口的兼容方法、分派入口与测试。
- 各接口独立维护版本分界；最新 else 默认处理当前及更新版本。
- 现有内部返回类型、Provider/Gateway 合同与原生项目归属语义不变。

## 非目标

不全量重构 Client/DTO/Mapper，不增加兼容注册表、整实例 CompatibilityPlan、完整接口 trait 或另一层统一模型；未变化接口不新增分支和委托。不新增公共字段，不在生产启动/请求路径强制导出 schema，不以超过验证版本阻断新版本，不自管或按 cwd 推断原生项目。

## 现状理解

`client.rs` 负责进程与 RPC，`client/projects.rs` 已集中项目调用，`protocol.rs` 定义官方 DTO，`mapper.rs` 转换为 Provider DTO，`provider.rs` 编排业务和生命周期；继续沿用这些边界。

官方 0.166.0-alpha.5.1 schema 的 Project.position 为必填整数；0.171.0-alpha.13 已移除。差异传递到 project/list、read、create、update 四个响应，请求参数与 delete 没有变化。两个版本的最小完整 ProjectList schema 已保留在 tests/fixtures/contracts/，新 DTO 的必需字段由契约测试校验。本次项目探测错误不再丢弃模型能力的修复继续保留。新会话首轮前可能尚未落盘，原生 ID 正确也不一定代表桌面分组已采用，见[跨端记录](../40-runbooks/codex-project-position-schema-drift.md#跨端归属验收发现的独立限制2026-11-25)。

## 实现路径

### 1. 最小模块结构

```text
src/client.rs                  # 共用 RPC、进程和当前进程版本
src/client/projects.rs         # 保留现有接口入口，按需要增加分派
src/client/compatible/
  mod.rs                      # 声明模块，不维护接口注册表
  version.rs                  # 完整版本比较工具
  vxxxx.rs                     # 只放本次变化接口
  vyyyy.rs                     # 只放本次变化接口
src/protocol.rs               # 沿用共用 DTO 和内部返回类型
src/mapper.rs                 # 沿用公共领域映射
```

实际模块为 `v0166_alpha5_1.rs`、`v0171_alpha13.rs`。本次四个项目入口各自使用 `<= 0.166.0-alpha.5.1` 和最新 else。旧版官方 binary 已通过 npm SHA512 完整性校验，两个版本均已导出实验 schema；基线由用户指定，表示旧实现支持范围假设，不等于穷举测试此前每个版本。position 首次移除的准确版本仍未知。未变化接口原地保留。

### 2. 每个接口独立分派

```rust
// 伪代码：真实阈值由 schema/样本确认。
fn project_list(client: &Client, input: Input) -> Result<ProjectPage> {
    let version = client.codex_version();
    if version <= PROJECT_LIST_OLD_UPPER {
        CompatibleVxxxx::project_list(client, input)
    } else {
        CompatibleVyyyy::project_list(client, input)
    }
}
```

下次该接口变化，把原最新分支封为 `else if version <= PROJECT_LIST_YYYY_UPPER`，再增加新的最后 else；其他分支保留。thread_list 只有自身变化才增加自己的分界，两个接口的分支数量和版本边界无需相同。

CompatibleVxxxx 只是变化接口实现的容器，不代表完整 Provider。某次仅 project_list 变化，就只实现 project_list；未变化的 thread_list 不新增方法、不增加版本判断、不委托旧结构体。不存在整个 Provider 一起切换兼容版本。

### 3. Client、DTO 和 Mapper 的职责

兼容方法封装对应接口的参数编码、响应解码和必要转换，调用共用 RPC。专用官方 DTO 放在该兼容模块；共用类型继续复用，结构真正不同时才拆 DTO，不将所有字段改成可选或逐个尝试解码器猜版本。

所有分支返回现有 CodexProjectPage、CodexConversationSnapshot 等内部类型。版本特有转换在兼容方法内完成，现有 mapper.rs 继续映射为同一种 Provider Project/Conversation，不引入版本分派。模块可见性保持在 Codex crate 内，不扩大 SDK 公共 re-export。

### 4. 版本范围与验证记录

分派条件是该接口版本范围的唯一执行依据，不重复维护 JSON 注册表。入口附近记录：官方方法名、上界常量、变化原因、schema/fixture 来源和已验证版本。上界对应旧实现最后适用版本，不能把首次观察到新结构的版本直接当成旧上界。

共用工具比较完整版本，保留 alpha.13 等预发布标识和数字段，不能截断或按字符串大小比较。会话使用实际运行进程确认的版本，沿用 initialize.userAgent 并结合 --version 核对；无法确定时明确诊断，不伪装成某版。磁盘升级不改写旧进程的版本，新进程重新读取。

最新 else 默认处理更高版本，不增加“超过已验证版本”门禁。验证记录只说明实际测试范围，不能声称未来版本已通过；小版本没改变接口就不新建兼容实现，回归后更新验证记录即可。

### 5. 新官方版本的维护流程

1. 在开发/CI 环境从指定 binary 导出稳定和实验 schema，留存完整版本及构建/schema 指纹。
2. 用小型脚本解析 $ref、必需关系和类型，按方法比较请求、响应与相关事件，过滤描述文字等无关差异。
3. 无相关变化就复用实现并回归；有变化才新增受影响接口的兼容方法、专用 DTO 和分界。
4. 保留旧样本测试；请求新增必填字段或默认行为改变不能简单认定为兼容，schema 相同仍需行为验证。

schema 检查是开发维护工具，不是生产调用前置依赖。最新分支实际失败时，返回现有标准错误和可选 details，记录接口、版本、所选实现，不打印业务 payload、不增加公共协议字段。写入已发送后不换实现重试；不支持的能力仍按现有能力发现与真实错误处理，不以默认向前调用伪造授权或方法可用性。

### 6. 原生项目归属与局部落地

各兼容实现保留完整 route 和 native ID；thread/start 显式传递所选 projectId，main/worktree 只影响执行目录，不改变项目身份。不得将项目请求静默改为 standalone、按 cwd 猜项目或写桌面私有状态。

沿用未落盘处理，创建不强制等待首次持久化。写入已发送但超时/解码失败时保留现有结果未知语义和可核对的 thread ID，不自动重新创建。原生 ID 正确不能代替原生 App 分组验收。

已补完整 SemVer 比较及四个项目接口的局部兼容。旧分支继续使用严格 i64 position；新 DTO 仅存在对应兼容模块，转换后仍返回同一 CodexProject。mapper.rs 保持原实现。schema 比较使用 scripts/diff_codex_schema.py，导出 JSON 自带相关 definitions，因此会连带报告被引用 DTO 变化；保留 default/required/type 等合同字段，过滤描述类注解。脚本只提供差异，不自动宣布语义兼容，本次不新增 CI 门禁。

## 涉及模块

| 模块 | 原因及范围 |
| --- | --- |
| Client 相关入口、新增 compatible 模块 | 每个变化接口独立选择实现，共用底层 RPC |
| protocol.rs / mapper.rs | 保留共用结构和领域映射，仅将版本专用 DTO/转换收进局部实现 |
| provider_vertical.rs、native_runtime.rs、fixture | 验证新旧分支与项目、写入生命周期 |
| schema 开发脚本及官方快照 | 定位引用 DTO 与方法差异，记录导出指纹；本次未新增 CI 门禁 |
| 公共 protocol/、SDK、Gateway、Remote | 合同及调用语义不变，作为回归对象，不增加官方版本判断或必需改动 |

## 风险

| 风险 | 验证路径 |
| --- | --- |
| 分界或预发布版本比较错误 | 测试等于上界、刚超过上界、alpha 数字段、正式版；边界以真实样本取证 |
| 最新分支遇到未来破坏性变更 | 新版 schema 差异及真实回归；错误不伪装成空数据或授权 |
| 共用类型调整踩坏旧接口 | 新旧 fixture 同跑，断言现有内部及公共合同输出 |
| 解码失败导致重复写入 | 断言不换实现重试，保留已有结果未知处理 |
| API 归属正确但桌面未采用 | 同 thread ID 验证 main/worktree 创建、原生分组与双向续聊 |

## 测试计划

每个变化接口补新旧 fixture、参数编码、返回转换及版本边界测试；未变化接口保持原测试。还须保留项目错误隔离、未知审批拒绝、generation/取消及未落盘回归。

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex
cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway --test builtin_provider_integration
npm run protocol:check
npm run providers:test
```

公共业务 schema 与生成的 SDK 运行时代码不变是检查项；SDK 的开发说明入口按用户要求从 README 改为 SKILL.md，并同步发布清单和导出器。真实非空项目只读检查与需要消息发送的跨端验收分开，后者在专用项目显式执行。以同一 thread ID 核对原生 projectId、原生 App 分组和反向续聊，覆盖 main/worktree，fixture 不能替代实机结论。

本次已执行新旧 DTO、预发布上界/紧邻版本/未来版本、四个接口、跨页与空续页、坏字段/重复游标/目标消失、项目异常保留聊天等回归；已执行真实新版非空项目只读探针。安装版 UI 与发送验收的具体证据见故障记录，不用 fixture 替代实测结论。

## 知识沉淀

方案遵守[项目身份规约](../60-rules/remote-project-identity.md)和[原生 API 兼容边界](../60-rules/provider-native-api-compatibility.md)。后续证据保留在接口注释、schema/fixture 和故障记录，不新增知识索引。兼容版本只组织变化接口，验证版本不作运行时上限。

## 未知项

position 首次移除的准确版本；同版本不同构建是否有契约差异；Windows 实机行为。旧 binary 已取得并导出 schema，上界采用用户指定的旧适配版本。默认向前调用不等于证明未来版本兼容。
