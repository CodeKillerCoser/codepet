# 本地开发、测试与构建

面向需要修改或构建 Code Pet 的开发者。命令均在仓库根目录执行。产品介绍与下载入口见 [README](../../README.md)。

## 环境要求

- 主要开发和完整功能验证平台是 macOS。项目包含 macOS 透明悬浮窗、Vision 抠图、Terminal/iTerm 激活和 DMG notarization 相关能力。
- Windows 已支持核心 Rust 编译检查和 Tauri 打包所需的 `.ico` 图标资源；macOS-only 能力会降级为明确的不支持提示。
- Linux 需要 Tauri/WebKitGTK/GTK 相关系统依赖。macOS 上交叉检查 Linux target 还需要配置对应 sysroot 和 `pkg-config`。
- Node.js 和 npm（Hook 接入运行时也需要 Node.js）。
- Bun：Provider staging 会调用它编译协议生成器；版本参考 `.github/workflows/release.yml`。
- Rust stable toolchain。
- Tauri 2 所需的本机构建依赖。
- 如需签名和 notarization，需要 Xcode 命令行工具、Apple Developer 证书和 notary credentials。

## 本地开发

安装锁定版本的依赖：

```bash
npm ci
```

启动前端开发服务器：

```bash
npm run dev
```

启动 Tauri 开发应用：

```bash
npm run tauri dev
```

`npm run tauri dev` 会先构建开发态 Provider adapter；首次启动需要 Rust 与 Bun 工具链。仅运行前端不能验证原生窗口、IPC 或 Agent 接入。

`npm run dev` 会在 `127.0.0.1:1420` 启动 Vite。Tauri 开发模式会自动使用这个地址。

## 测试

前端和 TypeScript 逻辑测试：

```bash
npx vitest run
```

Rust 测试：

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Provider/Host workspace 与 Codex Provider 生命周期垂直测试：

```bash
cargo test --manifest-path crates/Cargo.toml --workspace
cargo test --manifest-path crates/Cargo.toml -p codepet-provider-codex --test provider_vertical
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

Tauri bundle 会先构建并 staging Code Pet 自有的 Codex、OpenCode、Claude Provider adapter，再把 `provider-plugins/` 收录到 App Resources。它不会打包底层 Agent runtime；Codex/OpenCode/Claude 可执行文件仍来自用户本机检测或选择。仅验证 staging 可运行：

macOS 本地构建使用显式 ad-hoc bundle 签名，保证运行时 code-sign identifier 与 `CFBundleIdentifier` 都是 `com.codepet.desktop`，而不是随二进制哈希变化的 linker identity。ad-hoc designated requirement 仍随构建变化；需要让 Desktop/Documents 等 TCC 授权跨版本稳定时，必须改用同一 Apple Developer ID 证书签名。

```bash
npm run providers:test
npm run providers:stage
```

macOS universal 发布构建应设置 `CODEPET_PROVIDER_TARGET=universal-apple-darwin`；开发态可将 `CODEPET_BUNDLED_PROVIDER_PLUGINS_DIR` 设为 staging 目录的绝对路径。

生成并验证 macOS 签名 DMG：

```bash
npm run package:signed
```

`package:signed` 会执行 Tauri DMG 构建、codesign 校验、notarytool 提交、stapler 固化和 Gatekeeper 校验。它需要下列任一 notarization 配置：

- `CODE_PET_NOTARY_KEYCHAIN_PROFILE` 或 `APPLE_NOTARY_KEYCHAIN_PROFILE`
- 或 `APPLE_ID`、`APPLE_PASSWORD` / `APPLE_APP_SPECIFIC_PASSWORD`、`APPLE_TEAM_ID`
- 兼容旧变量：`APPLE_NOTARIZE_APPLE_ID`、`APPLE_NOTARIZE_PWD`、`APPLE_NOTARIZE_TEAM_ID`

签名脚本仅使用启动进程继承的环境变量，不会启动交互式 shell 或主动读取 shell 配置。请在调用前显式配置本项目使用的公证账号或 keychain profile；缺少配置时会在构建及清理旧 DMG 前退出。

## 项目结构

```text
.
├── .agents/skills/         # Codex 仓库级技能
├── frontend/               # Svelte 前端
│   ├── App.svelte           # 主窗口：Agent、用量、个性化、事件页
│   ├── PetApp.svelte        # 桌宠悬浮窗
│   └── lib/                 # API、活动归并、音效、宠物渲染、图表等
├── src-tauri/               # Tauri/Rust 后端
│   ├── hooks/               # 注入到各 Agent 配置里的 hook 脚本
│   ├── src/                 # 按功能域组织的 Rust 模块
│   │   ├── activity/        # collector、事件归一化、标题解析、Token 用量
│   │   ├── agent/           # Agent 注册、hook、交互控制和远程能力
│   │   ├── app/             # 设置、状态、日志、自启动和 CLI
│   │   ├── pet/             # 宠物库、抠图、主题默认值
│   │   └── platform/        # 平台窗口能力
│   └── tests/               # Rust 集成测试
├── crates/                  # Provider Host 与独立 Agent adapter
├── protocol/                # 协议定义
├── sdk/                     # 多语言 SDK
├── knowledge/               # 活知识库，目录树和标题即语义索引
├── scripts/                 # 打包签名辅助脚本
├── AGENTS.md                # AI Agent 协作入口指令
├── package.json             # npm 脚本和前端依赖
└── README.md
```

## 重要模块

- `src-tauri/src/agent/registry.rs`：Agent 列表、配置路径和 hook 事件声明。
- `src-tauri/src/agent/hooks.rs`：托管 hook 写入和移除逻辑。
- `src-tauri/src/activity/collector.rs`：本机 HTTP collector。
- `src-tauri/src/activity/events.rs`：hook payload 到桌宠事件的归一化。
- `src-tauri/src/app/state.rs`：近期事件、授权决策和 collector 共享状态。
- `src-tauri/src/app/settings.rs`：应用设置读写和默认值。
- `src-tauri/src/pet/library.rs`：宠物库、图片导入、像素化和宠物选择。
- `src-tauri/src/activity/token_usage.rs`：Token 用量解析和聚合。
- `src-tauri/src/app/log.rs`：应用日志、启动 banner 和性能事件记录。
- `src-tauri/src/app/autostart.rs`：基于 Tauri autostart 插件的登录启动控制。
- `src-tauri/src/agent/actions.rs`：任务卡片激活、回复和审批能力边界。
- `frontend/lib/activity.ts`：任务活动归并、过滤和展示辅助逻辑。
- `frontend/lib/agentInteractions.ts`：前端任务操作 capability 计算。
- `frontend/lib/sound.ts`：通知音效、鞭子音效和抽打反应音。

## 开发约定

- 不要在没有明确要求时运行格式化工具。
- 提交前确认 `git status`，只暂存本次相关文件。
- 功能变更优先补必要测试，但不需要为了每一行实现都写测试。
- README 中描述的功能应当和真实产品能力保持一致，避免写尚未实现的控件或流程。

## 维护依据

构建入口以 `package.json`、`src-tauri/tauri.conf.json` 和 `.github/workflows/release.yml` 为准；模块职责见 [运行拓扑](../10-architecture/runtime-topology.md)。发布与签名的完整检查见 [GitHub Release 更新发布](../40-runbooks/github-release-updates.md)。新增依赖时同步验证开发启动和安装包构建。
