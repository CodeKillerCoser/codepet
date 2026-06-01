# Code Pet

Code Pet 是一个面向本机 AI 编程工具的桌面宠物应用。它会常驻在桌面上，监听 Codex、Claude Code、Qoder 和 Cursor 的 hook 事件，把任务开始、工具调用、等待授权、失败和完成等状态变成可见的桌宠消息，并提供通知音效、宠物外观、任务气泡、Token 用量统计等个性化能力。

项目使用 Tauri 2、Svelte 5、Vite 和 Rust 构建。主窗口用于配置和查看数据，透明桌宠窗口用于日常悬浮展示。

## 功能概览

- **桌面宠物悬浮窗**：透明、置顶、可自动调整高度的桌宠窗口，用于展示最近的任务活动。
- **多 Agent 事件接入**：支持 Codex、Claude Code、Qoder 和 Cursor 的 hook 配置接入，托管 hook 命令兼容 Windows 和类 Unix shell。
- **任务状态卡片**：展示任务标题、工具调用、状态、时间、授权等待等信息。
- **授权提醒**：等待授权时可以响铃，并在用户处理前重复提醒；普通完成任务只提示一次。
- **通知音效**：支持内置通知音、静音、自定义通知音频和静音时段。
- **抽鞭子互动**：桌宠窗口提供抽鞭子按钮，点击后播放鞭子动画和鞭声；可配置被抽后的反应音，包括 `啪`、`啊啊啊` 和自定义音频。
- **宠物个性化**：支持默认像素宠物、图片导入、抠图导入、像素化程度调整和宠物库。
- **任务气泡样式**：支持主题、背景呼吸灯、边框跑马灯、多色渐变和动画速度设置。
- **Token 用量统计**：读取本机审计和 transcript 数据，按 Agent、时间范围和桶大小聚合展示 Token 用量。
- **最近事件日志**：在主窗口中查看近期收到的 hook 事件。
- **开机自启动**：通过 Tauri autostart 插件在设置页控制登录启动，覆盖 macOS、Windows 和 Linux 桌面平台。
- **启动性能日志**：记录后端启动阶段、Codex audit 回放、Token 用量刷新和前端首屏请求耗时，便于排查启动变慢。
- **macOS 打包签名辅助**：提供 DMG 构建、签名校验和 notarization 脚本。

## 支持的 Agent

| Agent | 配置文件 | 事件覆盖 |
| --- | --- | --- |
| Codex | `~/.codex/hooks.json` | `SessionStart`、`UserPromptSubmit`、`PreToolUse`、`PostToolUse`、`PostToolUseFailure`、`PermissionRequest`、`Stop` |
| Claude Code | `~/.claude/settings.json` | `SessionStart`、`UserPromptSubmit`、`PreToolUse`、`PostToolUse`、`PostToolUseFailure`、`PermissionRequest`、`Stop` |
| Qoder | `~/.qoder/settings.json` | `SessionStart`、`UserPromptSubmit`、`PreToolUse`、`PostToolUse`、`PostToolUseFailure`、`PermissionRequest`、`Notification`、`Stop` |
| Cursor | `~/.cursor/hooks.json` | `sessionStart`、`beforeSubmitPrompt`、`preToolUse`、`postToolUse`、`beforeShellExecution`、`afterShellExecution`、`beforeMCPExecution`、`afterMCPExecution`、`afterFileEdit`、`stop` |

启用某个 Agent 时，应用会把托管的 `code-pet-hook.mjs` 写入对应配置。关闭时会移除托管项，并清理该 Agent 的当前事件。托管命令使用 `node <script> --agent <id>` 形式传递 Agent 信息，并保留对旧版 `CODE_PET_AGENT=...` 托管项的识别和升级能力。

## 环境要求

- 主要开发和完整功能验证平台是 macOS。项目包含 macOS 透明悬浮窗、Vision 抠图、Terminal/iTerm 激活和 DMG notarization 相关能力。
- Windows 已支持核心 Rust 编译检查和 Tauri 打包所需的 `.ico` 图标资源；macOS-only 能力会降级为明确的不支持提示。
- Linux 需要 Tauri/WebKitGTK/GTK 相关系统依赖。macOS 上交叉检查 Linux target 还需要配置对应 sysroot 和 `pkg-config`。
- Node.js 和 npm。
- Rust stable toolchain。
- Tauri 2 所需的本机构建依赖。
- 如需签名和 notarization，需要 Xcode 命令行工具、Apple Developer 证书和 notary credentials。

## 本地开发

安装依赖：

```bash
npm install
```

启动前端开发服务器：

```bash
npm run dev
```

启动 Tauri 开发应用：

```bash
npm run tauri dev
```

`npm run dev` 会在 `127.0.0.1:1420` 启动 Vite。Tauri 开发模式会自动使用这个地址。

## 使用方式

1. 启动应用。
2. 在主窗口的 `Agent` 页启用需要接入的工具，例如 Codex 或 Claude Code。
3. 运行对应 Agent 的任务。
4. 桌宠窗口会展示任务状态、工具调用、授权等待、完成或失败等活动。
5. 在 `个性化` 页调整宠物、主题、任务气泡、通知音效和抽打反应音。
6. 在 `用量` 页查看按 Agent 聚合的 Token 使用情况。
7. 在 `最新事件` 页查看最近收到的 hook 事件。

### 抽鞭子互动

桌宠窗口右侧有抽鞭子按钮。点击后：

1. 播放鞭子抽打动画。
2. 播放鞭子抽打音效。
3. 如果配置了抽打反应音，会在鞭声后继续播放桌宠反应音。

在 `个性化 -> 通知声音 -> 抽打反应` 中可以选择：

- `无`：只播放鞭声。
- `啪`：播放短促拍打反应。
- `啊啊啊`：播放内置叫声。
- `自定义`：选择本机音频文件作为桌宠被抽后的反应音。

自定义反应音和普通通知自定义音是两套独立配置。

### 通知和重复提醒

通知声音支持：

- `blip`
- `chime`
- `bell`
- `custom`
- `silent`

完成、失败、授权等待可以分别控制是否响铃。等待授权属于需要人操作的状态，在用户处理前可以重复提醒；普通任务完成只提示一次。

## 本地数据

应用会在本机写入少量状态和缓存：

- 应用设置：系统 local data 目录下的 `code-pet/settings.json`。
- Token 用量缓存：系统 local data 目录下的 `code-pet/token-usage.json`。
- 应用日志：系统 local data 目录下的 `code-pet/logs/code-pet.log`。
- 宠物库：默认位于系统 local data 目录下的 `code-pet/pets`，也可以在设置页修改。
- 离线事件暂存：`~/.code-pet/spool/events.jsonl`。

Tauri asset protocol 允许读取 `$APPLOCALDATA`、`$DATA`、`$LOCALDATA`、`$HOME` 和 `$TEMP` 范围内的图片或音频资源，用于宠物图片和自定义音效。

## Collector

应用内置一个本地 collector：

```text
http://127.0.0.1:47621/hook
```

hook 脚本会把 Agent 事件转发到这个地址。前端在浏览器预览无法使用 Tauri IPC 时，也会尝试读取：

```text
http://127.0.0.1:47621/events
```

collector 只绑定 `127.0.0.1`。

## Token 用量统计

Token 用量统计会读取本机 Agent 审计和 transcript 信息，并生成聚合摘要。当前默认关注：

- `~/.codex/audit/audit.jsonl`
- `~/.qoder/audit/audit.jsonl`
- 审计记录中引用的 transcript 文件

主窗口的 `用量` 页可以选择时间范围和桶大小，查看各 Agent 的用量分布。

## 性能和日志

应用启动时会写入可读的日志 banner，用于区分新日志文件、轮转后的日志文件和每次应用启动。性能事件以 `[perf]` 日志行记录，格式为可解析的 key-value，例如：

```text
[perf] name=startup.total status=ok duration_ms=123 agents=4
```

当前覆盖的主要性能点包括：

- 后端启动总耗时、宠物悬浮窗配置、Agent 配置读取、离线事件回放。
- Codex audit 读取、最近记录截取、transcript 解析和历史事件回放。历史回放会在启动 setup 完成后异步执行，避免阻塞窗口启动。
- Token 用量刷新、audit 引用 transcript 数量、递归扫描 transcript 数量和文件体积。
- 主窗口首次刷新、设置读取、Agent 列表、事件快照、宠物库、Token summary、开机自启动状态读取。
- 桌宠窗口设置读取、ready 时间和最近事件同步耗时。

## 测试

前端和 TypeScript 逻辑测试：

```bash
npx vitest run
```

Rust 测试：

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Windows 目标编译检查：

```bash
cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc
```

签名打包脚本测试：

```bash
python3 scripts/package_signed_test.py
```

项目当前没有 `npm run lint` 脚本。

## 构建

构建前端资源：

```bash
npm run build
```

构建 Tauri 应用：

```bash
npm run tauri build
```

生成并验证 macOS 签名 DMG：

```bash
npm run package:signed
```

`package:signed` 会执行 Tauri DMG 构建、codesign 校验、notarytool 提交、stapler 固化和 Gatekeeper 校验。它需要下列任一 notarization 配置：

- `CODE_PET_NOTARY_KEYCHAIN_PROFILE` 或 `APPLE_NOTARY_KEYCHAIN_PROFILE`
- 或 `APPLE_ID`、`APPLE_PASSWORD` / `APPLE_APP_SPECIFIC_PASSWORD`、`APPLE_TEAM_ID`
- 兼容旧变量：`APPLE_NOTARIZE_APPLE_ID`、`APPLE_NOTARIZE_PWD`、`APPLE_NOTARIZE_TEAM_ID`

## 项目结构

```text
.
├── src/                    # Svelte 前端
│   ├── App.svelte           # 主窗口：Agent、用量、个性化、事件页
│   ├── PetApp.svelte        # 桌宠悬浮窗
│   └── lib/                 # API、活动归并、音效、宠物渲染、图表等
├── src-tauri/               # Tauri/Rust 后端
│   ├── hooks/               # 注入到各 Agent 配置里的 hook 脚本
│   ├── src/                 # collector、settings、pets、token usage 等模块
│   └── tests/               # Rust 集成测试
├── scripts/                 # 打包签名辅助脚本
├── package.json             # npm 脚本和前端依赖
└── README.md
```

## 重要模块

- `src-tauri/src/agents.rs`：Agent 列表、配置路径和 hook 事件声明。
- `src-tauri/src/hooks.rs`：托管 hook 写入和移除逻辑。
- `src-tauri/src/collector.rs`：本机 HTTP collector。
- `src-tauri/src/events.rs`：hook payload 到桌宠事件的归一化。
- `src-tauri/src/state.rs`：近期事件、授权决策和 collector 共享状态。
- `src-tauri/src/settings.rs`：应用设置读写和默认值。
- `src-tauri/src/pets.rs`：宠物库、图片导入、像素化和宠物选择。
- `src-tauri/src/token_usage.rs`：Token 用量解析和聚合。
- `src-tauri/src/app_log.rs`：应用日志、启动 banner 和性能事件记录。
- `src-tauri/src/autostart.rs`：基于 Tauri autostart 插件的登录启动控制。
- `src-tauri/src/activity_actions.rs`：任务卡片激活和回复能力，macOS 终端自动化与跨平台路径打开边界。
- `src/lib/sound.ts`：通知音效、鞭子音效和抽打反应音。
- `src/lib/activity.ts`：任务活动归并和展示辅助逻辑。

## 开发约定

- 不要在没有明确要求时运行格式化工具。
- 提交前确认 `git status`，只暂存本次相关文件。
- 功能变更优先补必要测试，但不需要为了每一行实现都写测试。
- README 中描述的功能应当和真实产品能力保持一致，避免写尚未实现的控件或流程。
