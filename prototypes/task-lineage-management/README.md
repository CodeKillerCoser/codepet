# 任务谱系管理交互原型

这是 `knowledge/20-product/task-lineage-management.md` 的可交互桌面端原型，用于验证：

- 主对话与 Fork、新对话、Subagent 的层级导航；
- 对话视角与 Task 视角切换；
- Task 自由图和时间泳道图；
- 节点路径高亮、同 Thread 节点联动和右栏消息回溯；
- 主工作区/worktree、提交状态和同步状态的紧凑表达；
- 中栏与右栏复用同一个消息列表组件。

原型使用模拟数据，不包含 Transcript 扫描、AI 抽取、JSON 持久化或真实 Thread 消息发送。目标实现方案见 `knowledge/10-architecture/task-lineage-and-extraction.md`。

## 本地运行

```bash
npm install
npm run dev -- --host 0.0.0.0 --port 4173 --strictPort
```

## 验证

```bash
npm run build
npm run test:sites
```

视觉截图保存在 `qa/`，完整对照结果见 `design-qa.md`。
