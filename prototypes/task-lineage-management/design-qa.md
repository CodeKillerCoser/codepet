# Design QA

## Source of truth

- Base graph visual: `/Users/wangxin/.codex/generated_images/01a06f66-089b-7483-8bb8-7ecc09fbcf69/exec-a4fb0797-f0ad-44d0-b6c8-08ff49da7207.png` (1487×1058)
- Base swimlane visual: `/Users/wangxin/.codex/generated_images/01a06f66-089b-7483-8bb8-7ecc09fbcf69/exec-d876cae5-ba08-4060-ab7f-84ff45957e53.png` (1536×1024)
- Current user corrections are authoritative over the earlier visuals: project-scoped expandable main-conversation trees; conversation-first center view; task view only on main conversations; task-node work mode, commit state, and main-workspace synchronization; one shared message-list component.

## Implementation evidence

- Conversation-first view: `/Users/wangxin/Documents/Codex/2026-09-05/ni/codepet-task-lineage-prototype/qa/implementation-conversation.png` (1440×1024)
- Graph view: `/Users/wangxin/Documents/Codex/2026-09-05/ni/codepet-task-lineage-prototype/qa/implementation-graph.png` (1440×1024)
- Swimlane view: `/Users/wangxin/Documents/Codex/2026-09-05/ni/codepet-task-lineage-prototype/qa/implementation-swimlane.png` (1440×1024)
- CSS viewport: 1440×1024; device pixel ratio: 1.
- Density normalization: implementation captures share the same viewport and density. The 1487×1058 graph source is treated as a responsive composition reference, while the latest written corrections define the changed information architecture.
- Default state: project `codepet`; first main conversation expanded and selected; center mode `对话`; right column shows conversation facts and three related tasks.

## Findings

- No actionable P0/P1/P2 issues remain.
- Typography: the existing Inter/SF Pro Text/PingFang hierarchy is preserved. Main conversation, child conversation, metadata, task node, and code-state levels remain visually distinct without oversized labels.
- Spacing and layout: the tree indentation, message column, task canvas, and inspector align to the established compact three-column grid at 1440×1024. No desktop clipping is visible.
- Colors and tokens: blue continues to represent selection/current flow; violet represents derived context/same thread; green represents ended, committed, or synchronized; amber represents pending, uncommitted, or unsynchronized.
- Image and icon fidelity: this interface contains no raster product imagery. Phosphor icons consistently represent main conversation, Fork, new conversation, Subagent, work mode, commit, and synchronization.
- Copy and content: `独立 worktree`/`主工作区`, `已提交`/`未提交`, and `已同步`/`未同步`/`无需同步` are separate, observable concepts. Child rows expose title and creation method without invented roles.

## Full-view comparison evidence

- The original three-column structure remains intact in all three captures.
- The left column now represents the requested hierarchy: only main conversations appear at level one, and the expanded first main conversation exposes Fork, new-thread, and Subagent children.
- The default center view is the selected conversation's message flow. Only the selected main conversation exposes the `任务` switch; a selected child conversation exposes only `对话`.
- Task mode retains the three horizontal task options, graph/swimlane switch, branching and convergence, hover path behavior, and right node inspector.
- Every graph and swimlane node now carries work mode, commit state, and synchronization state without requiring a click.

## Focused comparison evidence

- Conversation tree: connectors, caret controls, indentation, provenance icons, titles, task counts, and selected state remain legible within the 232px sidebar.
- Shared messages: the same `MessageList` component is visibly used as the center conversation stream and the right task-node stream; both include the same author row, message surface, composer, Enter-to-send behavior, and send button.
- Node footer: three compact factual states fit on one aligned line in both task visualizations. A worktree example shows `独立 worktree · 已提交 · 未同步`, while a main-workspace example shows `主工作区 · 未提交 · 无需同步`; a synchronized worktree example is also present.
- Right inspector: work mode, workspace path, branch, commit hash, and synchronization to the main workspace are readable and aligned before the reusable message stream.

## Interaction verification

- Main conversations expand and collapse; derived conversations are selectable.
- Selecting a derived conversation displays its own messages and removes the task-view option.
- Selecting a main conversation defaults to its message flow and offers conversation/task switching.
- Switching to task mode preserves the horizontal task selector and graph/swimlane controls.
- Clicking a task node selects it persistently; hover no longer changes the right inspector's committed selection.
- Sending from the center message stream appends the message and changes that conversation context to `运行中`.
- Sending from the right task-node stream appends the message to the selected node and derives the task as `进行中`.
- Source inspection confirms one `MessageList` definition with two render sites: center conversation detail and right node inspector.
- Build passed and all four Sites packaging tests passed.
- Console was checked after a clean reload at `2026-09-05T08:16:07.159Z`; no new warnings or errors were emitted.

## Comparison history

1. [P1] The conversation list was flat and mixed main and derived conversations. Fixed with a project-scoped expandable tree whose first level is main conversations and whose children expose creation method.
2. [P1] Selecting a conversation led directly to task visualization. Fixed with conversation-first routing; only main conversations can switch to task mode.
3. [P2] Task nodes exposed thread and commit state but not work mode or synchronization. Fixed with a three-part factual node footer and matching inspector facts.
4. [P2] Center and right message streams duplicated markup and behavior. Fixed with one reusable `MessageList` component used by both surfaces.
5. [P2] Child titles initially collapsed to a few characters because an absolutely positioned connector consumed a grid column. Fixed by reducing the child row to the two real content columns; post-fix capture shows full titles.
6. [P2] Hover-driven inspector preview made the selected node unstable during message composition. Fixed by keeping hover for path/thread highlighting only and binding the right inspector to persistent click selection.
7. Post-fix visual and interaction evidence confirms all six issues are resolved without removing the established task tabs, graph, swimlane, or direct messaging behavior.

## Follow-up polish

- P3: React Flow attribution remains visible as required by the library.
- P3: The prototype remains desktop-first; at very narrow panel widths the right inspector overlays the center rather than becoming a separate route.

final result: passed
