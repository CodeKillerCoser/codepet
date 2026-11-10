# 顶部面板 UI CI 的键盘展开超时

## 现象与证据

提交 `c394192` 的 [Pet UI quality 运行](https://github.com/CodeKillerCoser/codepet/actions/runs/42687493241) 在 `pet_notch_ui_qa.mjs` 的 Tab 展开步骤超时，41 项单元测试通过。此前本机使用已安装 Chrome，CI 使用 Playwright Chromium。

本机改用固定依赖提供的 Chromium 后复现同一失败。按 Tab 前 `document.activeElement` 为 body、`document.hasFocus()` 为 true；超时后 activeElement 仍为 body，但 hasFocus 为 false，frame 维持 closed。说明 Tab 导航离开网页，组件没有收到聚焦事件。设置明确起点后，同一键盘用例通过；继续回归发现圆角检查读取到动画中间值 `10.4805px`，而用例期待终点 `12px`。

## 根因及引入历史

该 CI 随 `c394192` 引入。测试把 body 是 activeElement 当作 Tab 从文档开头进入的保证，但 Chromium 会保留此前鼠标交互留下的顺序导航起点。该起点可能已经位于面板之后。另有布局断言仅等待正文可见，没有等待展开动效结束。旧本机浏览器与 CI 不一致，掩盖了这些测试前置条件和时序问题。

## 修复与涉及模块

- `scripts/pet_notch_ui_qa.mjs`：在面板前插入测试专用焦点起点，从那里实际按 Tab，并断言头像获得 `:focus-visible`；不直接聚焦面板来绕过键盘导航。检查静态布局前等待组件 phase 与 morph 状态稳定。失败时保存截图、焦点、窗口和 DOM 信息。
- 两个顶部 QA 脚本：默认使用依赖提供的 Chromium，与 CI 保持一致，仍可通过环境变量指定浏览器。
- 生产组件和状态机未因本次测试故障修改；保留焦点、Escape、布局和逐帧位移断言，不通过放宽超时或跳过用例处理。

## 验证与回归防线

执行 `npx playwright install chromium`、`npm run test:pet:ui`、`npm run test:pet:motion`，再执行 `uv run --with pillow python scripts/pet_motion_images_qa.py`。推送后必须检查同一提交在 Linux CI 的结果，不能用本机 Chrome 通过代替。本机完整 Chromium UI 回归、8 种连续帧场景、内容更新与浏览器像素检查已通过。失败诊断会随 `pet-ui-regression` 产物上传。

## 未知项

浏览器 UI 检查不覆盖 macOS 最终桌面合成；原生验收边界见[原生动效排查](pet-native-motion-flicker.md)。
