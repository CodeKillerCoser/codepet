# 对话内三档半屏终端

## 背景与目标

2026-11-28 用户要求在手机对话界面内打开终端，以业界标准组件提供小半屏、大半屏、全屏；电脑终端输出什么，手机就呈现什么。此前 Remote 没有交互式终端服务，工具调用结果不能替代真实 PTY。

本实现创建由 CodePet Host 持有的真实 PTY，通过原有已认证 Gateway 通道附着；手机按字节顺序将输出交给终端模拟器。这里的同步指同一个 PTY 的控制序列和内容，不是桌面窗口的像素截图。现有 Terminal/iTerm 窗口没有通用的外部附着协议，本轮不读取它们的屏幕、滚屏或历史。

## 产品行为

- 对话页工具栏的“电脑终端”打开面板，初始小半屏约 35%，大半屏约 70%，全屏为安全区内 100%。Flutter 官方 Material Bottom Sheet 与 `DraggableScrollableSheet` 负责路由、拖动和吸附，界面不展示尺寸选择按钮，尺寸切换主要通过手柄；标题栏只有紧凑的全屏/还原图标，退出全屏回到先前半屏尺寸。手柄提供辅助功能的增减操作。短屏或键盘出现时以控制区的最小可用高度调整小档比例。
- 只用顶部手柄拖动面板；终端自身的滚动、选择和输入由 xterm 处理。尺寸改变不重建终端模型，列数/行数反向同步给 PTY。中文输入、颜色、光标、回车覆盖和备用屏幕交给 VT 模拟器；不拆成日志行，不去除 ANSI。
- 同一设备、同一对话在手机进程内复用一个 controller 和 PTY，工作目录优先使用已加载会话的 workspaceRoot，未提供时使用电脑用户目录。不同对话分别持有会话。
- 收起、退到后台、暂时断线都不等于结束 PTY。重新打开复用模型与字节偏移，健康设备重连后只重试读取。“结束终端”明确关闭；“新建终端”先确认关闭旧会话再创建新键和模型。终端结束后仍可查看手机已有输出。
- 快捷键 Esc、Tab、Ctrl+C 和方向键由终端模拟器按当前输入模式编码。输入串行、每块至多 16 KiB、待发送至多 64 KiB。任何输入确认不确定都会停止后续输入，不自动重放命令。

## 实现路径与模块

```text
ConversationDetailScreen → SnapBottomSheet → TerminalView
        DeviceSession → TerminalSessionController（VT 模型、UTF-8、偏移、写入顺序）
               → TerminalGateway / ProtocolTerminalGateway
               → generated Gateway SDK → 已认证 LAN / RTC
Host Gateway terminal dispatcher → TerminalManager → portable-pty → 用户 shell
```

- Remote `features/common/snap_bottom_sheet.dart`：标准通用容器，独立于终端；使用既有 Material 主题与布局 token。
- `features/terminal/conversation_terminal_sheet.dart`、`ui/theme/app_terminal_theme.dart`：终端视图、快捷键、局部状态与共享字号适配。
- `application/terminal/terminal_session_controller.dart` 与 `terminal_input_handler.dart`：跨面板保存纯 Dart VT 状态；流式解码处理跨块 UTF-8，epoch 丢弃旧会话迟到回包；输出失败不改变设备连接或 Provider 状态。xterm 4 的 keytab adapter 将 AppCuKeys 错接到 appKeypadMode，因此本地输入 adapter 使用真实 DECCKM 处理无修饰的方向/Home/End，剩余输入继续走库的默认 handler；回归分别开启光标模式与数字键盘模式。
- `protocol/v2/gateway/terminal/`：`terminal.open/read/write/resize/close` 的唯一合同。`protocol.describe` 广告 `terminal-pty-v1`，旧 Host 显示升级提示。终端属于 Host，不伪装成 AI Provider，也不往 channel-control 塞业务方法。业务版本保持兼容区间，增量能力由 feature 发现。
- Host `terminal/{mod,output,platform}.rs`：会话与授权归属、环形输出及平台 shell 选择。复用 WezTerm 的 `portable-pty 0.9.0`，Unix 使用有效的 SHELL 或 `/bin/sh`，Windows 使用 COMSPEC；业务调用方没有平台分支。
- Host `gateway/terminal.rs`：以通道注入的 caller scope 隔离会话，OS 操作放 blocking worker，3 秒操作预算；写入超时只清理对应 PTY。Gateway 原长文件按 server trait 实现与 DTO 映射拆分为 `gateway/server.rs`、`gateway/mapping.rs`，保持公开 re-export 和旧逻辑。

## 生命周期、边界与风险

| 风险 | 约束和验证路径 |
| --- | --- |
| 高频输出拖慢连接/事件总线 | 不把输出发布到全局 replay；每次读取至多 32 KiB、同会话至多一个读取在途，空闲轮询约 120 ms，有输出约 16 ms；Host 心跳和 Provider 查询有并发回归。 |
| 手机暂离后无限堆积数据 | 每 PTY 保留至多 1 MiB；偏移过期返回明确局部错误，停止附着并由用户新建，不悄悄丢字节后继续解释损坏的 VT 状态。手机滚屏保留 10000 行。 |
| 多端越权或进程资源泄漏 | caller 来自认证 credential；仅该 caller 能读写/关闭本终端。每 caller 最多 8 个、Host 最多 32 个。运行中 PTY 只在显式关闭/Host 退出时终止；已完成且 30 分钟未访问的会话可回收。 |
| 输入重试导致命令重复执行 | 输入序号从 1 递增，仅相同末次序号与字节可去重确认；客户端写入失败后停止，不自动重试。测试覆盖串行发送和未知结果。 |
| 已退出 PID 被误用 | wait/kill 共享进程句柄锁，回收后清空句柄；close/drop 不对已回收 PID 再发信号。真实 shell 退出、重复关闭和 shutdown 测试覆盖。 |
| 小屏/IME 挤掉动作 | 320×800、800×360、1000×900，含真实路由的键盘 inset；验证吸附、手柄、关闭、焦点、终端模型不重建以及尺寸上报。 |

会话和 VT 模型尚未持久化到手机磁盘；不承诺手机进程被杀后自动恢复同一终端，也不承诺 Host 重启后恢复 shell。普通断链与页面收起的恢复已覆盖。此限制不能用协议读取重试冒充进程级恢复。需要跨进程持久会话时，可另行接入 tmux/会话清单，但不能凭空复原已丢弃的 VT 状态。

## 验证与验收

本地自动化包含真实 shell、Rust Gateway RPC、Dart 生成 SDK 到真实 Host/PTY/VT 的跨语言链路，以及三档布局与生命周期测试。相邻用例还验证实际对话页的草稿、连接状态及光标应用模式。`tests/fixtures/terminal_probe.rs` 只用于 stdio 测试桥；生产仍走已认证的 LAN/RTC 通道。

```sh
cargo test --manifest-path crates/Cargo.toml -p codepet-host --lib --test manager_gateway --test remote_lan_listener --test terminal_gateway -- --test-threads=1
cargo build --manifest-path crates/Cargo.toml -p codepet-host --bin codepet-host-terminal-probe
# 在 Remote 仓库运行；路径指向上一步实际输出文件
CODEPET_TERMINAL_PROBE=/path/to/codepet-host-terminal-probe flutter test --no-pub
flutter analyze --no-pub
npm run protocol:check
npm run check:source-size
```

Tauri 直接 cargo 检查沿用既有 `TAURI_CONFIG='{"app":{"macOSPrivateApi":false},"bundle":{"resources":[]}}'`，不修改生产配置。需要同时部署新版 Host 和 Remote。尚未执行 Android/iOS 真机的输入法、蜂窝网切换、后台挂起、复杂 TUI 或 Windows 原生 shell 验收；本地渲染与 Mac PTY 成功不代表这些项目已经通过。

### 初版验证记录（2026-11-28，移除尺寸按钮之前）

实际工作区为 Remote `/Users/wangxin/Documents/codepet-remote`，基线 `9b26742eacb5425099ae3f3e45660f0eb1a37391`；Host 为本仓库工作区，基线 `e8353232c8fede054235b4b8f6aefb97122aa3bf`。先在最新主分支的隔离工作树验证，再将相关改动合回此工作区并重新检查；既有用户文档改动保持不变。

- Remote 全量 542 项通过（设置真实 Host probe 路径，跨语言用例没有跳过）；`flutter analyze --no-pub` 无问题。
- Host 当前工作区 lib 115 项、manager_gateway 59 项、LAN/RTC 4 项通过，另有 2 项原有现场探针按约定忽略；terminal_gateway 最终 3 项通过，包括一个 PTY 持续洪泛输出时的真实健康终端执行及心跳。6 个 PTY 单元用例包含活动 shell 不因手机离线而被回收。
- Rust Gateway SDK 7 项、协议 freshness/合同 41 项通过；Remote 三份 SDK 同步检查通过。Tauri 使用上述既有检查配置编译通过，保留原有 warning。
- 源码尺寸检查 0 违规、0 审查项；Remote 改动手写 Dart 文件均低于 1500 行；两仓库 diff whitespace 检查通过。
- Android arm64 Debug `0.1.3+4` 编译与签名验证通过，buildRevision 为 `9b26742-terminal-sheet-wip-20261128`，APK SHA-256 为 `68e4811e9e980e5dbdcac81501b35e2f505981c886706207a882f8afde5c660d`。这是当前工作区调试构建，未安装或发布。

日志、9 张三档布局预览、安装包与精确补丁快照保存在 `artifacts/terminal-sheet-20261128/`。预览是 Widget 测试渲染，使用本地测试字体，不冒充实体手机截图。正式手机/跨网络验收仍按上节执行；本轮未自动推送代码或创建 Release。

### 面板视觉修订（2026-11-28）

用户指出三档大按钮占用界面，已删除整排分段选择。小/大半屏由官方手柄拖动吸附，全屏与还原使用标题栏单个图标，辅助功能手柄继续提供增减操作；全屏后还原至先前的小/大半屏。短屏最小高度从 200 降至 160，保留控件可用性并让出内容空间。外层 route 不强制圆角，内层随全屏状态调整。

本次重新验证 320×800、800×360、1000×900，覆盖无尺寸按钮、小/大半屏拖动、全屏/还原、键盘/缩放/焦点、对话草稿和同一 PTY 模型；相关 8 项测试、静态分析及源码尺寸通过。新预览与日志保存在 `artifacts/terminal-sheet-chrome-20261128/`。前节安装包和补丁为初版快照，不包含此次视觉修订；本轮未重新构建或发布安装包。

## 来源与后续观察

标准组件见 [Flutter DraggableScrollableSheet](https://api.flutter.dev/flutter/widgets/DraggableScrollableSheet-class.html)、[Material Bottom Sheet](https://api.flutter.dev/flutter/material/showModalBottomSheet.html)；终端组件见 [xterm.dart](https://pub.dev/packages/xterm)、[portable-pty](https://docs.rs/portable-pty/latest/portable_pty/)。当前锁定 xterm 4.0.0，后续升级需比较维护分支的中文输入、CJK 宽度、备用屏与原始控制序列兼容，不只比较版本号。

连接与终端局部状态继续遵守 [连接与业务隔离](../60-rules/remote-connection-business-isolation.md)。不新增第二套通道准入、Provider 能力表或中央知识索引。
