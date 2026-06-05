# 项目知识入口

这个目录是长期项目事实的入口。它说明 Code Pet 是什么，以及后续 Agent 在改代码前应该从哪里开始阅读。

## 优先阅读

- `product-intent.md`：产品目的、用户对象和非目标。
- `../10-architecture/README.md`：运行形态和模块边界。
- `../30-domains/README.md`：按功能域组织的更深实现事实。
- `../40-runbooks/README.md`：可复用的排查流程。
- `../60-rules/README.md`：可复用开发约束。

## 证据来源

- `README.md`：面向用户的能力和本地开发命令。
- `frontend/`：Svelte UI、任务卡片行为、设置界面、主题 token 和声音逻辑。
- `src-tauri/src/`：Rust 后端、collector、设置持久化、Agent hook 和平台行为。
- `src-tauri/tests/` 与 `frontend/**/*.test.ts`：已受测试保护的行为。

## 维护规则

保持目录树语义化。不要新增 `map.yaml` 或类似中心化索引；未来 Agent 应该通过目录名和文档名理解知识库结构。
