# 产品意图

Code Pet 是以远程控制和 Provider 插件扩展为核心的 AI Agent 平台。电脑端通过多进程 Host / Provider 架构接入本机工具，配套 CodePet Remote 提供手机上的设备、项目、会话与任务操作入口。桌宠是本地提醒和个性化的附加体验。

## 产品目标

- 通过 Remote 远程创建或继续会话、查看实时输出，并按 Provider 能力处理任务与审批。
- 允许开发者实现兼容的 Provider 协议插件，在 Host 加载后复用 Remote 的标准功能，无需针对每种 Agent 重做客户端。
- 以 Gateway、独立 Provider 插件进程和本机 runtime 分工，隔离原生工具差异与客户端业务。
- 作为附加体验，展示已接入来源的本地任务活动，让用户不必一直盯着每个 Agent 窗口。
- 突出需要注意的状态，例如权限请求、失败和任务完成。
- 当后端具备可靠 provider 能力时，在任务卡片上提供轻量操作。
- 允许用户个性化宠物外观、任务气泡样式、声音和桌宠窗口透明度。
- collector 流量保持本机内闭环；collector 只绑定 `127.0.0.1`。

## 非目标

- Code Pet 不是替代 Agent 的运行时。
- 不把特定 Provider 的能力扩大为全部工具均支持；协议未覆盖的新能力仍需两端共同扩展。
- 不把局域网控制宣传成已完成的公网中继，也不把多进程隔离宣传成权限沙箱。
- README 不承担完整知识库职责。

## 当前证据

- `README.md` 面向使用者介绍远程控制、插件扩展、技术选型和多进程架构；技术细节留在领域文档。
- [双端架构](../10-architecture/host-remote-platform.md) 记录 Host / Remote 文档核对来源、进程边界与当前限制。
- `tools/cp-sdk-gen/README.md` 提供独立 Provider 的 SDK 导出、接口实现、manifest 安装与验证说明。
- `src-tauri/src/activity/collector.rs` 将 collector 绑定到 `127.0.0.1:47621`。
- `frontend/PetApp.svelte` 渲染透明桌宠窗口和任务卡片操作。
- `frontend/App.svelte` 渲染主配置界面。

## 未知项

- Hanging Metal 与 Code Pet 的长期产品命名尚未在代码注释和文档中完全统一。
