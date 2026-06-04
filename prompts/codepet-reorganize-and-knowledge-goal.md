# Goal Prompt: Code Organization And Living Knowledge Base

请进入 goal 模式，完成 Code Pet / Hanging Metal 项目的代码组织整理与活知识库基础文档落地。

目标：
先完成当前仓库的代码目录重组，使前端和 Tauri 后端代码组织更清晰；在代码整理完成并验证通过后，再创建项目级活知识库与 Agent 开发规约文档体系。最终目标是让项目结构更适合后续 AI Agent 开发、排查和知识沉淀。

重要澄清：
1. 先做代码整理，再做知识库文档。
2. 不要提交或推送，除非我明确要求。
3. 不要引入重 CI/CD。
4. 不要使用中心化 `map.yaml` 或类似映射文件；知识库目录树和文档标题就是语义索引。
5. 当前用户提到的 “Torii” 应先结合仓库实际路径判断；如果仓库实际是 Tauri 代码，应按 `src-tauri` 处理。
6. 不要丢失已有功能，不要为了整理目录做无关重构。
7. 每一步都要尽量保持可验证、可回滚、可解释。

阶段 0：前置检查

1. 检查当前工作区状态：
   - `git status --short --branch`
2. 阅读当前仓库结构：
   - `README.md`
   - `package.json`
   - `vite.config.ts`
   - `tsconfig.json`
   - `index.html`
   - `pet.html`
   - `src/`
   - `src-tauri/src/`
   - `src-tauri/tests/`
3. 检查是否已有：
   - `AGENTS.md`
   - `CLAUDE.md`
   - `.claude/`
   - `skills/`
   - `knowledge/`
4. 如果发现非本任务产生的未提交改动，先识别并避免覆盖。必要时向我说明。

阶段 1：前端目录重命名

目标：
把当前前端代码目录从 `src/` 改名为 `frontend/`。

要求：
1. 将根目录下的 `src/` 移动为 `frontend/`。
2. 更新所有引用旧路径的地方，包括但不限于：
   - `index.html`
   - `pet.html`
   - `tsconfig.json`
   - `vite.config.ts`
   - 测试文件引用
   - README 中项目结构说明
   - 任何 `src/`、`src\\`、`src/lib`、`src/PetApp` 等路径文本
3. 保持前端内部相对 import 尽量不变。
4. 不改变业务行为。
5. 完成后使用 `rg` 搜索旧路径残留，并判断哪些是合理保留、哪些必须更新。

建议验证：
- `npx vitest run`
- `npm run build`

阶段 2：Tauri 后端代码二级目录整理

目标：
当前 `src-tauri/src/` 下模块较平铺，需要按照功能域整理为二级目录，降低后续 AI Agent 理解成本。

要求：
1. 先阅读现有 Rust 模块依赖关系，不要直接机械移动。
2. 设计一个低风险的二级目录方案。
3. 优先保持公共模块语义清晰，避免过度拆分。
4. 移动文件后更新：
   - `src-tauri/src/lib.rs`
   - `mod.rs`
   - Rust `use crate::...` 路径
   - 测试中的模块引用
   - README / knowledge 中涉及的路径
5. 尽量通过 `mod.rs` 做清晰聚合，让目录树表达功能域。

推荐目标结构可以参考，但不必机械照抄：

```text
src-tauri/src/
  agent/
    mod.rs
    registry.rs
    control.rs
    hooks.rs
    actions.rs
    codex_app_server.rs
    codex_audit.rs
    claude_transcript.rs

  activity/
    mod.rs
    events.rs
    collector.rs
    title_resolver.rs
    token_usage.rs

  app/
    mod.rs
    log.rs
    state.rs
    cli.rs
    autostart.rs

  pet/
    mod.rs
    library.rs
    subject_cutout.rs
    theme_defaults.rs

  platform/
    mod.rs
    macos_window.rs

  lib.rs
  main.rs
```

注意：
- 可以根据实际依赖调整命名。
- 如果某个模块移动成本过高，可以保留在原位置，但必须说明原因。
- 不要为了目录美观破坏已有 crate API。
- 不要把所有东西塞进一个 `common` 或 `utils`。

建议验证：
- `cargo test --manifest-path src-tauri/Cargo.toml`
- 如果完整 Rust 测试耗时过长，至少运行能覆盖模块编译的命令，并说明未完整执行的原因。
- `npx vitest run`
- `npm run build`
- `git diff --check`

阶段 3：更新 README 中的项目结构

目标：
代码目录改名和后端重组后，README 不能过时。

要求：
1. 更新 README 的项目结构部分。
2. 更新重要模块路径。
3. 不要把 README 扩写成知识库。
4. README 仍保持项目入口文档定位。

阶段 4：创建 Agent 协作入口 `AGENTS.md`

目标：
创建一个简洁的 Agent 工作协议入口。

要求：
1. `AGENTS.md` 不承载大量项目知识。
2. 它只说明 Agent 在本项目中开发前、开发中、开发后应该遵守的协议。
3. 至少覆盖：
   - 先读相关源码和测试。
   - 根据语义检索 `knowledge/`。
   - 识别影响范围和旧风险。
   - 修改后运行相关测试。
   - Bug 修复后沉淀知识。
   - 重复 Bug 或通用约束升级为开发规约。
   - 不要把重要结论只留在聊天里。
4. 内容保持短，可执行，不写口号。

阶段 5：创建语义化 `knowledge/` 目录树

目标：
创建项目级活知识库。目录树和文件标题本身就是事实和索引，不使用 `map.yaml`。

要求：
1. 创建语义清晰的目录结构。
2. 每个目录要有入口 `README.md`。
3. 文档内容要基于当前代码、README、测试和实际实现。
4. 不要写空泛百科，不要写尚未实现的功能为事实。
5. 可以先写“最小完整版本”，但关键领域必须有入口和核心事实。

建议结构：

```text
knowledge/
  00-project/
    README.md
    product-intent.md

  10-architecture/
    README.md
    runtime-topology.md
    frontend-backend-boundary.md
    event-pipeline.md
    settings-persistence.md
    window-system.md
    agent-control.md
    cross-platform-boundaries.md

  20-product/
    README.md
    pet-window-experience.md
    task-card-interaction.md
    personalization.md
    notification-and-approval.md

  30-domains/
    agent-events/
      README.md
      hook-ingestion.md
      event-normalization.md
      activity-merge.md
      known-risks.md

    agent-control/
      README.md
      codex-app-server.md
      qoder-remote-control.md
      capability-boundaries.md
      known-risks.md

    pet-window/
      README.md
      layout-and-sizing.md
      multi-monitor-bounds.md
      reply-editor.md
      known-risks.md

    settings-and-personalization/
      README.md
      settings-model.md
      pet-personalization.md
      notification-settings.md

    theme-system/
      README.md
      token-model.md
      migration-rules.md

  40-runbooks/
    README.md
    hook-not-working.md
    reply-not-shown-in-app.md
    window-overflows-screen.md
    task-list-collapsed-or-clipped.md

  50-decisions/
    README.md
    codex-app-server-as-primary-reply-path.md
    qoder-existing-session-reply-unsupported.md
    semantic-file-tree-as-knowledge-index.md

  60-rules/
    README.md
    bug-to-rule-promotion.md
    conditional-render-focus.md
    module-reorganization.md
```

阶段 6：使用已有文档写作技能

目标：
用前面已经创建的 `skills/living-dev-doc-writer/` 作为写作约束。

要求：
1. 撰写知识库文档时遵守该 skill 的原则：
   - 证据优先。
   - 涉及模块说明原因。
   - 风险对应验证。
   - Unknowns 不编造。
   - Bug 或约束可升级为 rules。
2. 如果发现该 skill 内容不足，可以只做小范围补充；不要把具体项目事实写死进 skill。

阶段 7：验证

代码整理完成后必须验证：

1. 前端：
   - `npx vitest run`
   - `npm run build`

2. Rust / Tauri：
   - 优先运行 `cargo test --manifest-path src-tauri/Cargo.toml`
   - 如完整测试耗时异常，说明原因，并至少完成能验证模块编译和关键路径的替代检查。

3. 文档：
   - 检查 `AGENTS.md` 存在且简洁。
   - 检查 `knowledge/` 目录结构语义清楚。
   - 检查没有 `map.yaml`。
   - 检查 README 路径没有明显过时。
   - 检查知识库没有把未实现能力写成已实现事实。

4. Git：
   - `git status --short`
   - `git diff --check`
   - `git diff --stat`

最终输出要求：
1. 完成内容摘要。
2. 前端目录重命名说明。
3. Tauri 后端目录重组说明。
4. 新增/更新的知识库文档列表。
5. 运行过的验证命令和结果。
6. 未完成或有意不做的部分。
7. 风险和后续建议。
8. 明确说明没有提交或推送。

验收标准：
1. 前端代码目录从 `src/` 改为 `frontend/`。
2. 项目能通过前端测试和构建。
3. Tauri 后端代码不再全部平铺在 `src-tauri/src/`，已有功能通过验证。
4. README 中项目结构和重要模块路径已更新。
5. 存在简洁的 `AGENTS.md`。
6. 存在语义化 `knowledge/` 目录树。
7. 知识库包含架构、产品、领域、runbook、decision、rules 等入口文档。
8. 不存在中心化 `map.yaml`。
9. 没有引入重 CI/CD。
10. 没有提交或推送。

/goal
