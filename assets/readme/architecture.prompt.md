# README 架构图生成记录

- 方式：内置 image_gen，生成后按中文、多进程、前端和跨平台要求迭代编辑。
- 最终资产：`architecture.png`。
- 依据：`knowledge/10-architecture/host-remote-platform.md`。
- 人工核对：中文标签、Svelte 前端、Tauri IPC、Rust 主进程中的 Gateway/Provider Host、独立插件及 runtime 子进程、SDK 依赖、独立桌宠链路。
- 手机画面为示意，不是产品截图；跨平台是设计目标，平台交付范围以 README 文字为准。

## 最终编辑提示词

Use case: infographic-diagram
Edit this existing Chinese Code Pet architecture graphic. Preserve the Chinese labels, tasteful light blue-white palette, navy typography, smartphone left, plugin processes center-right, runtime right, SDK foundation, and secondary local pet. Make a targeted correction: the desktop main application region is missing the FRONTEND.

Widen the desktop application panel as necessary, increase canvas space as needed; keep everything readable and clean, no overcrowding. Inside the currently labelled "桌面主进程" region, replace its heading with "桌面应用". Directly below title add a clearly visible horizontal top card labelled "前端界面" and smaller "Svelte 5 / Vite · 主窗口 / 桌宠窗口". Under this card a bidirectional vertical connector labelled "Tauri IPC". Below that, a nested explicitly bounded panel labelled "Rust 主进程 · Tauri / Tokio", containing existing "Gateway" / "统一业务入口" and "Provider Host" / "插件发现 · 实例管理 · 请求路由" modules side by side. This makes frontend visible while precisely placing Gateway and Provider Host in the Rust process. Add subtle small readable note beneath desktop application panel "WebView 由系统托管" so the frontend WebView is not claimed to physically run inside Rust process.
Remote arrow must still terminate at Gateway, not frontend. Provider protocol arrow originates at Provider Host. Keep plugin processes and runtime child process boundary. Rename subtitle "远程控制 · 跨平台设计 · 多进程插件架构". Bottom SDK and pet remain. All explanatory labels Chinese; preserve product/technical names. No iOS support claims, cloud, internet relay, sandbox claims or fabricated features. Correct spelling, typographic hierarchy, generous whitespace, polished GitHub README architecture illustration.
