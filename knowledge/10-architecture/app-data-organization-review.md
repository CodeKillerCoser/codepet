# 应用数据目录盘点与整理方案

> 本文保留调整前的磁盘盘点。用户已选择直接切换新目录，不做迁移或兼容；现行读写规则见 [Path Manager](path-manager.md)。

## 背景

2026-09-13 对本机 Code Pet 数据做只读盘点。当前工作树为 `deb34110`，安装版为 `D:/Software/Code Pet/code-pet.exe`，文件版本 `0.3.9-beta`。二者功能差异很大，不能以当前工作树没有某模块为由删除对应数据。另读取主仓库 `D:/17633/Documents/Code/codepet`（HEAD `28aefff7`，含未提交 Path Manager 等改动）核对新模块；这些源码不等于已确认的安装版构建来源。

## 目标

明确文件的功能归属、保留策略及渐进整理方式。

## 非目标

本次不删除、迁移用户数据，不修改业务代码，不修改主仓库已有改动。

## 现状理解

固定入口 `%LOCALAPPDATA%/code-pet/settings.json` 的 `data.dataDirectory` 和 `petLibrary.dataDirectory` 均为 null，当前数据根即默认目录。下表容量来自运行期间文件枚举，后续会变化，单位为 MiB/KiB。

| 实际文件/目录 | 数量、约容量 | 应用功能与生命周期 |
| --- | --- | --- |
| settings.json | 1，2.1 KiB | 设置页偏好与数据目录引导入口；保留 |
| pet-sources.json | 1，44 B | Agent 页宠物活动来源启停偏好；保留 |
| logs/ | 18，73.56 MiB | 主程序诊断和事件日志页；可以按明确保留策略清理历史 |
| provider-host/ | 7，48.5 KiB | 设备身份、Provider 实例映射、Host 会话状态与旧备份 |
| providers/ | 9，617.9 KiB | Claude/Codex/OpenCode 各自的 data/provider.sqlite、锁和 logs/provider.log；数据库含业务状态，不能整体当缓存 |
| remote-access/ | 3，6.7 KiB | lan-tls-identity.json 为局域网 TLS 身份；remote-credentials.json 为配对/访问凭据；rtc-cloud.json 为云中继配置与身份材料。必须保留并限制访问 |
| skills/ | 2，2.1 KiB | extract-tasks、reconcile-tasks 的用户可编辑 SKILL.md；不能当自动生成缓存删除 |
| spool/events.jsonl | 1，387 B | Hook 未送达事件缓冲；消费成功后清理，不按文件年龄删除 |
| task-lineage/v1/<hash>/ | 3，232 B | 抽取配置 extraction-settings.json 与 writer/inference 锁；本次没有观察到任务树等成果文件 |
| workspaces/task-extraction/run-LAB1Ef/ | 5，4.2 KiB | 一次抽取的 config、input、prompt、result 与 Skill 快照；主仓库任务抽取文档明确记录为合成输入实测回执 |
| webcontent/v1–v4/ | 48，2.69 MiB | 独立前端安装包，包含 HTML、JS/CSS/音效与 manifest；不是网页缓存 |

数据根共约 77.0 MiB；其中 `logs/code-pet.log.1` 约 49.1 MiB，约占根目录 64%。事件日志有五个 UUID 归档及各自 `.idx`，另有当前文件及索引；代码已实现约 5 MiB/分片、保留五份归档，不能把这些 UUID 文件误判为重复文件。`.idx` 是历史查询索引，生命周期应跟随其 JSONL。

额外位置：`%LOCALAPPDATA%/com.codepet.desktop/EBWebView` 有 329 个文件，约 32.88 MiB，为 WebView2 运行环境数据；不应整目录作为普通缓存清理。`~/.codepet/workspaces` 存在但本次未发现文件；`~/.code-pet` 本次未发现。安装目录中的 Provider 程序与包内 webcontent 属于程序资源，不计入数据根。

`provider-host` 同时存在 conversation-state.json、其锁、pre-sdk-v1.bak、conversation-state.sqlite 及其锁。新源码以 SQLite 为主且会按 Provider 路由，但不能仅凭扩展名证明旧 JSON 已完整迁移。零字节锁文件也不证明没有进程持锁；盘点时 App 与三个 Provider 进程均在运行。

v1/v2 的 manifest 没有 builtAt，v3/v4 有。主仓库新加载器在包内与数据目录中选择兼容、完整的最高语义版本，同版本比 builtAt；vN 是槽位，不代表版本高低。尚未确认当前进程实际加载哪个槽位，旧日志只证明曾加载 v1。

## 实现结果

在 ea9ee1e1 基础上修改读写代码，采用 config、connections、providers、tasks、pets、logs、versions、cache 的功能目录。详细文件归属、模块职责与验证路径已更新到 Path Manager、设置持久化、任务抽取运行时与 webcontent 文档。

用户明确不要求迁移或兼容，因此不实施之前建议的迁移备份与版本接管。旧磁盘数据保持原样，本次只调整仓库代码。日志保留策略与数据管理 UI 不属于本次目录调整范围，仍为后续建议。

## 未知项

原安装版的准确源码与当前 UI 槽位未进一步核对；本次未替换安装版。盘点容量是运行期间快照，不代表固定占用。
