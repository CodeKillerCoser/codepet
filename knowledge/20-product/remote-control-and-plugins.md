# 远程控制与 Provider 扩展

## 产品定位

Code Pet 是运行在电脑上的 AI Agent Host，与 [CodePet Remote](https://github.com/CodeKillerCoser/codepet-remote) 组成远程控制平台。远程操作和插件扩展是主线；桌宠是本地附加体验。

## 使用流程

1. 安装 Code Pet 和需要的本机 Agent runtime，完成工具自身登录与配置。
2. 在电脑端“连接 → 本机运行时”检查 Provider 检测结果、可执行文件选择和连接状态。内置 adapter 随安装包提供，底层 runtime 不随包安装。
3. 根据 Remote 仓库说明获取 Android APK。两端处于可互通的局域网，打开电脑端连接页添加设备。
4. Remote 发现 Host 后发起配对，核对两端数字码，在电脑端确认；二维码也是可用入口。发现只提供候选，配对完成才建立持久信任。
5. Remote 选择设备和 Provider，浏览项目/最近会话，创建或继续会话，发送内容并查看流式输出。审批、中断与运行中调整取决于 Provider 与任务能力。
6. 不再允许某台设备访问时，在电脑端撤销对应凭据；客户端需要重新配对。当前准入是设备级，不提供逐 Provider ACL。

Remote 下载与构建入口以其 README 为准，不假设总有 Release APK。当前主要工程为 Android；跨局域网中继、WebRTC、文件传输和网页预览不属于当前已交付上手流程。

## 开发自己的 Provider

Provider 作者负责原生工具适配，Remote 负责标准交互。先按 [SDK 接入指南](../../tools/cp-sdk-gen/README.md) 导出匹配版本的 Rust Provider server SDK，实现初始化、描述、实例生命周期和需要的业务方法，并发布标准事件。

将插件可执行文件和 `codepet-provider.json` 放入应用数据目录的 `provider-plugins/<插件目录>/`，或使用设置 `providerPlugins.directories` 配置额外搜索目录。manifest 声明稳定身份、可执行入口和实例；详细字段及验证命令以 SDK 指南为准。

加载插件后，Remote 经 Gateway 发现 Provider 并读取能力，现有标准操作可复用既有界面。新增原生工具不要求 Remote 直接连接原生接口；新增协议外功能则需要协议与客户端共同演进。

## 体验边界与验证

- 接入成功不等于工具可执行：检查本机 runtime 登录、版本和 Provider 实例状态，再发送真实任务验证。
- 插件在线不等于 Harness 正在运行：实例按连接与活动状态启停，不能把空闲停止显示成永久故障。
- 当前 Host 在手机离线后保留进行中任务和待审批；电脑/Host 必须继续运行。验证手机断线重连与任务结束，不能以关闭 Host 代替网络断线测试。
- Provider 不支持的方法不可伪装成成功；测试能力缺失、加载失败和版本不兼容时的界面提示。
- 配对流程需在真实电脑和 Android 设备验证；本次文档整理未执行双机验收。

## 依据

电脑端入口为 `frontend/App.svelte` 与 `ProviderConnectionStatus.svelte`；双向配对见 [配对设计](lan-peer-confirmed-pairing.md)，进程和协议见 [双端架构](../10-architecture/host-remote-platform.md)。本页整理既有能力，不提出新的实现方案。
