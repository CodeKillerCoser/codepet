# Runtime 漏检与最低版本约束

## 现象

2026-09-08，chensi-3 上 Code Pet 0.3.9-beta+f068a7c 自动选择 npm Codex 0.148.0，Remote 初始化会话列表报 `Codex Provider does not advertise ProjectList`。Host 安装列表缺少其他版本，Claude/OpenCode 显示未找到兼容运行时。

## 证据

- 手机日志 03:42–03:43 UTC：30.45.179.43 握手和 provider.describe 成功，conversation.list 失败；不是直接 project.list 失败。
- SSH 进程父子关系确认 CodePet 启动 npm Codex 0.148.0。启用 experimentalApi 后，该二进制 project/list 返回 -32600 unknown variant；threadSection/list 只返回 Pinned。ChatGPT 内置 Codex 0.153.4 的 project/list 返回 12 个项目。
- CLI 实测 Claude 2.1.142、OpenCode 1.18.26 可执行。已安装 Provider 通过 Host 协议扫描 Claude，在约 5.03 秒后返回 installed=[]、scanError=null。指定实际 executable 后，同一 Provider 的初扫和重扫成功。
- Python 复现 shell：不读取 stdout，等待 6 秒仍未退出；开始读取后正常结束。直接并发读取只需约 1.2 秒。基线 SDK 的大启动输出测试在 5.03 秒失败；修复后通过。
- 修复代码的远端原生 inventory-only 测试分别在约 2.24/2.15/2.45 秒完成 Claude/Codex/OpenCode 扫描与选择。Codex 同时报告 0.148.0 与 0.153.4，测试显式设置最低 0.151.0 时，前者标注不兼容并拒绝选择，后者可选择；这只是配置机制验证，不是默认下限。

## 根因

shell PATH 探测先等待退出，再读取 stdout。管道写入阻塞时形成循环等待，5 秒预算到期后静默返回无结果。GUI 精简 PATH 本身没有 npm 路径，因此 Claude/OpenCode 全部漏检。另一个独立缺陷是 shell PATH 仅返回第一个匹配项，会漏掉同一命令的其他安装。Codex 可从固定应用候选找到桌面内置版，但未配置版本下限时也可能选择旧 npm 版。

项目能力错误还有两层独立原因：Codex 0.148.0 返回 -32600 unknown variant，但 Provider 原先只识别 -32601 method not found，导致连模型及发送能力的整份 catalog 更新一起丢弃。Remote 在无项目能力时仍请求 standalone 范围，该范围要求 ProjectList；初始化异常处理再将列表失败提升为整个连接失败。

修复后，Provider 按被探测的方法精确识别旧版 unknown variant 错误，保留聊天能力，只取消项目能力；Remote 根据项目能力选择 standalone 或 all 范围，后续分页沿用同一范围。真正的参数错误不冒充“方法不支持”。有项目能力时仍保留聊天、项目、最近的展示结构。

## 引入历史

先等退出再读输出以及只取首项出现在此前 local_runtime 实现中；最初引入提交未完整追溯。当前失败安装版为 f068a7c。

## 修复方案与涉及模块

- Provider SDK local_runtime：并发读取 shell 输出，保留 5 秒预算与进程树清理，输出上限 1 MiB；完整枚举 shell PATH，按 canonical 路径去重。保留显式候选的路径错误和 shell 探测失败信息。
- Provider 协议 RuntimeInstallation：增加可选 minimumVersion、incompatibilityReason，安装列表包含检测到但不满足下限的版本。runtime.inventoryChanged 使用同一 DTO，Host 不重新检测产品版本。
- 三个 Provider：自动选择跳过不兼容安装，手选和带路径的实例配置也校验约束。配置通过 Provider manifest 的 env.CODEPET_RUNTIME_MIN_VERSION 传入；使用标准 semver 比较，预发行版本不能冒充同号正式版。
- 三个 Provider 保留可配置最低版本机制，默认不设未经验证的下限。最低版本只能约束必要的基础协议；Project API 是可选能力，缺少该接口时从能力列表移除项目方法，不能因此拒绝整个运行时。
- Tauri Host 和安装列表：展示检测数量、每份安装的版本/路径/下限/原因，低版本按钮禁用；所有安装均不兼容时不显示可用。

配置示例：在 Provider 的 codepet-provider.json 中设置 `"env": { "CODEPET_RUNTIME_MIN_VERSION": "0.151.0" }`，重启对应 Provider 生效。删除该键恢复无版本下限。这里的版本仅为配置示例；不能按可选功能的引入版本设置整个 Provider 的最低版本。

## 验证结果

- 项目降级回归覆盖 -32601 与旧版 -32600：没有 ProjectList，仍有发送能力，all 会话列表可读取；分类单测拒绝把其他参数错误当成方法缺失。
- SDK local_runtime 测试覆盖下限、边界、预发行、无效配置、重复路径、多个安装、大段 shell 输出与取消。
- 三个 Provider 单元测试、Host cargo check、前端构建、协议检查和 9 项前端 runtime 测试通过。
- UI 用两份 Codex 安装数据验证旧版禁选、新版可选、焦点及 600px 窄窗口无横向溢出。
- 远端修复版 Provider 原生扫描与选择测试通过；仅 inventory-only，不声称执行模型或完整实例启动已验证。
- OpenCode vertical 首次并发执行有一项计数断言失败；该项在基线和最终代码中单独复跑均通过。
- Codex 全量 vertical 有历史请求顺序断言失败；未修改基线也出现同类失败，涉及后台 thread/list/loaded/list 插入及严格请求序列。不能将全套报告为通过。
- 远端修复版独立 Provider 通过 Host mux 的探针出现 3 秒握手超时；原生扫描测试与安装版显式路径测试通过，完整新安装包验收仍需补充。

## 当前电脑处理

已备份 settings.json 并显式选择 ChatGPT 内置 Codex，以及 npm Claude/OpenCode 的真实 executable，重启 CodePet。保留原安装程序，未把临时测试二进制替换进已签名 App。新安装列表和自动发现修复尚需新版本安装包。

## 回归防线与未知项

更新 provider-local-runtime-paths 规约。保持大输出测试与多安装测试，Windows 走独立发现实现；Windows 实机尚未验证。持续观察 shell 真实失败时 scanError 是否可见，避免把空列表统一解释为版本不兼容。
