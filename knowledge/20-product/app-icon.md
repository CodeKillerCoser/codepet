# 应用图标

## 背景与目标

Code Pet 与 CodePet Remote 共用用户选定的黑猫白手套图：C 形卷尾、琥珀色眼睛、米白背景。原图保存在 `assets/branding/codepet-icon-source.png`，由内置 imagegen 生成并经用户确认。导出只缩放与转换格式，保留构图及颜色。

## 实现与涉及模块

- `src-tauri/icons/`：桌面安装包使用的 PNG、Windows ICO、macOS ICNS 与 iconset；原有 `src-tauri/tauri.conf.json` 引用继续生效。
- `scripts/generate_app_icons.py`：使用 Pillow 缩放，使用 macOS `iconutil` 打包 ICNS。执行 `python3 scripts/generate_app_icons.py` 更新桌面资源。
- CodePet Remote 的 `android/app/src/main/res/mipmap-*/ic_launcher.png`：Android 启动图标，覆盖 mdpi 至 xxxhdpi；其 Manifest 继续引用 `@mipmap/ic_launcher`。

同步 Remote 时执行 `python3 scripts/generate_app_icons.py --remote-root /path/to/codepet-remote`，同时复制原图到 Remote 的 `assets/branding/`。无需把此源图声明为 Flutter 运行时资源。

## 风险与验证

- 小尺寸可能损失白爪与眼睛细节：检查 32px 桌面图标及 48px Android 图标。
- 格式或密度错误：解码 PNG、ICO、ICNS，检查尺寸及现有配置引用；Android 构建检查资源可被打包。
- 操作系统可能缓存旧图标：安装新构建后检查 Dock 和启动器；资源验证不能替代实际安装验证。

## 范围

本次接入静态应用图标，不涉及桌宠皮肤、Provider 标识或动态状态图标。Remote 当前仓库仅有 Android 平台目录，其他平台接入时应从同一原图导出。
