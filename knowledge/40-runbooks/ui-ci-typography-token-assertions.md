# 主题字号调整后 UI CI 的标题断言失败

## 现象与证据

2026-11-27，主干 df656eb7 的 [Pet UI quality](https://github.com/CodeKillerCoser/codepet/actions/runs/45857095433) 在 `npm run test:ui` 失败：抽取历史弹窗标题实际 15px，测试固定期待 18px。前面的 Svelte 检查、桌宠单元测试和三个桌宠浏览器回归均通过。56592082 起的多次运行同样失败；该提交的 macOS 构建成功，不应将此 UI 测试故障描述为原生构建失败。

本机使用依赖提供的 Chromium 复现相同断言。`ExtractionHistory.svelte` 使用 `font: var(--font-section)`，主题的 section → lg → 15px 符合新的层级；脚本中“使用共享标题字体”的说明与固定 18px 的断言不一致。

## 根因与引入历史

56592082 统一了字号 token，`--font-size-lg` 从 18px 改为 15px，但 `scripts/ui_components_qa.mjs` 的标题断言没有同步。它验证了旧尺寸，而非声明的共享 token 约束。

## 修复与涉及模块

- UI QA：在同一主题作用域建立临时参考元素，应用 `--font-section`，比较实际标题的字号、字重及行高，并单独确认 token 已定义。参考元素随后移除。既不把期待值改成另一个固定数字，也不跳过标题检查；产品样式不变。
- Pet UI workflow：把 `scripts/ui_components_qa.mjs` 纳入 push/PR 路径过滤，确保测试自身变化也受检；截图/回执按脚本当前的 Chromium/WebKit 子目录上传，失败诊断亦保留。

## 验证与回归防线

本机先复现失败，再运行 `npm run test:ui` 的 Chromium 与 WebKit 回归，覆盖主题、尺寸、设置保存、反复切页、配对与焦点、抽取历史和更新弹窗。执行 source-size 与 diff 检查。推送后应确认同一提交的 Linux Pet UI quality 全部步骤完成；历史失败回执保留，不靠重新运行旧提交解决错误断言。

主题调整应验证语义样式角色；仅在产品明确固定尺寸时才断言具体像素。修改 QA 路径或输出目录时，同时核对 workflow 的 paths 与上传 globs。

## 未知项

此 CI 覆盖隔离 UI fixture，不代表已安装 macOS App 的完整人工验收；本次不改变窗口、任务、主题或原生构建行为。
