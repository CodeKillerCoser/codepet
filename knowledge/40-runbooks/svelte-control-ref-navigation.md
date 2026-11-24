# 连接接入切页中断与 Svelte 控件引用

## 现象

2026-11-24，用户反馈进入「连接接入 → 设备」后无法继续点击切换。

## 证据

- Chromium、WebKit 的主窗口 QA fixture 中，先进入远程设备，再切到本机运行时，均捕获 `props_invalid_value: Cannot do bind:ref={undefined} when ref has a fallback value`，调用栈指向 `Button` 和 `App.svelte`。
- 单独进入空设备列表和已有设备列表时未捕获该错误；已确认的失败点是随后挂载本机运行时页。不能据此断言用户现场只有这一条触发路径。
- 新增 `button.test.ts` 在修复前稳定失败：父组件未初始化引用，挂载 Button 时触发相同异常；初始化为 null 的对照组通过。
- Svelte 的 `reactivity/props.js` 会拒绝把 undefined 双向绑定到带非 undefined fallback 的 rune prop。

## 根因

本机运行时的 `providerAddButton` 在挂载前是 undefined。原生 `bind:this` 允许这种状态；迁移后的 `Button bind:ref` 使用 `$bindable(null)`，使挂载抛错并中断渲染。Input、Textarea 的同类封装也存在相同限制。

## 引入历史

提交 `72480902` 将运行时添加按钮从原生 `bind:this` 改为共享 Button 的 `bind:ref`，并引入带 null fallback 的组件。原 UI 回归只从活动接入跳至远程设备，没有进入本机运行时和反复切页，漏掉了该组合。

## 修复方案与涉及模块

- `frontend/lib/ui/{button,input,textarea}`：DOM 引用改用无 fallback 的 `$bindable()`，保留可选 ref API 和原有样式。
- `frontend/qa/BoundControlsQa.svelte` 与 `button.test.ts`：覆盖 undefined/null 初始引用以及三轮卸载、重新挂载，验证三个控件的引用更新和清理。
- `frontend/qa/main-window.ts` 与 `scripts/ui_components_qa.mjs`：空列表、已有设备各反复切换三轮，离开连接页后重新进入，并验证弹窗重新打开、关闭清理和焦点恢复。支持 Chromium 与 WebKit。

## 验证结果

- `npx vitest run frontend`：初次修复 215 项通过；接入 `fe5dddd4` 的最新 main 后 220 项通过。
- `npm run check:frontend`：整合后 0 错误、6 条既存警告。
- `npm run build:webcontent`、`env TMPDIR=/private/tmp npm run test:webcontent`：构建和 11 项资源测试通过。
- `npm run test:ui`、`CODEPET_QA_ENGINE=webkit npm run test:ui`：两种引擎通过，包含小窗口溢出、键盘 tab、弹窗焦点、设置持久化。
- 同两种引擎访问 Vite 开发入口，重复切换设备/运行时并打开、关闭配对弹窗：无页面异常。
- `npm run check:source-size`：0 违规、0 待拆分项；`git diff --check` 通过。

## 回归防线与规约候选

已将共享 DOM 引用的要求沉淀为 [Svelte 可绑定 DOM 引用](../60-rules/svelte-bindable-dom-refs.md)。组件库升级需要运行上述两种浏览器回归；只验证首次进入页面不足以覆盖条件挂载。

## 未知项

浏览器 QA 使用模拟 Tauri IPC，没有操作用户真实设备凭据；尚未在原生 Tauri 窗口复验。若真实窗口仍卡住，应继续捕获它的错误栈和具体切页顺序，区分渲染异常、遮罩残留和后端响应问题。
