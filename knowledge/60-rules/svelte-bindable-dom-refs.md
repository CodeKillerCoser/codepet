# Svelte 可绑定 DOM 引用

## 规则

封装原生元素的可选 DOM ref 必须接受挂载前的 undefined，并在卸载时清理。Button、Input、Textarea 使用无 fallback 的 `$bindable()`，不强迫所有调用方先初始化为 null。

## 适用场景

原生 `bind:this` 迁移为共享组件 `bind:ref`，以及条件页面、弹窗或控件反复挂载。

## 反例

父页面 `let button: HTMLButtonElement` 配合组件 `ref = $bindable(null)`，会在挂载时抛出 `props_invalid_value`，使当前渲染中断。

## 推荐做法

原生控件 ref 声明为可选，组件内部直接 `bind:this={ref}`；允许 undefined/null 初始值和卸载后重建。业务值的默认值应按实际语义保留。Bits UI 的代理属性应遵守其绑定契约，不能机械地移除所有 fallback。

## 来源

[连接接入切页中断复盘](../40-runbooks/svelte-control-ref-navigation.md)，引入提交 `72480902`。

## 验证方式

`npx vitest run frontend/lib/ui/button/button.test.ts` 检查三个原生控件引用的初始值和重挂载。`npm run test:ui` 及 `CODEPET_QA_ENGINE=webkit npm run test:ui` 检查空/有设备列表的反复切页、返回主界面、弹窗清理与焦点恢复；`npm run check:frontend` 检查公开类型契约。
