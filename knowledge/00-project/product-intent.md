# 产品意图

Code Pet 是面向本机 AI 编程工具的桌面宠物伴侣。它通过透明悬浮桌宠窗口把后台 Agent 活动变成可见状态，同时用普通主窗口承载配置和数据查看。

## 产品目标

- 展示 Codex、Claude Code、Qoder 和 Cursor 的任务活动，让用户不必一直盯着每个 Agent 窗口。
- 突出需要注意的状态，例如权限请求、失败和任务完成。
- 当后端具备可靠 provider 能力时，在任务卡片上提供轻量操作。
- 允许用户个性化宠物外观、任务气泡样式、声音和桌宠窗口透明度。
- collector 流量保持本机内闭环；collector 只绑定 `127.0.0.1`。

## 非目标

- Code Pet 不是替代 Agent 的运行时。
- 在 provider 控制路径被验证前，不声明主动控制能力。
- README 不承担完整知识库职责。

## 当前证据

- `README.md` 记录了支持的 Agent、本地 collector 端点、设置数据和构建/测试命令。
- `src-tauri/src/activity/collector.rs` 将 collector 绑定到 `127.0.0.1:47621`。
- `frontend/PetApp.svelte` 渲染透明桌宠窗口和任务卡片操作。
- `frontend/App.svelte` 渲染主配置界面。

## 未知项

- Hanging Metal 与 Code Pet 的长期产品命名尚未在代码注释和文档中完全统一。
