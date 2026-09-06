# Prototype Instructions

Run the local server yourself and open the preview in the browser available to this environment. Do not give the user server-start instructions when you can run it.

Before making substantial visual changes, use the Product Design plugin's `get-context` skill when the visual source is unclear or no longer matches the current goal. When the user gives durable prototype-specific design feedback, preferences, or decisions, record them in `AGENTS.md`.

When implementing from a selected generated mock, treat that image as the source of truth for layout, component anatomy, density, spacing, color, typography, visible content, and hierarchy.

Build app UI in `src/`. Keep `.openai/hosting.json`, `worker/index.js`, `scripts/prepare-sites-build.mjs`, and `tests/sites-worker.test.mjs` intact so the same local prototype can be handed to Sites. Before a Sites handoff, run `npm run build` and `npm run test:sites`; the build must leave `dist/client/index.html`, `dist/server/index.js`, and `dist/.openai/hosting.json`.

## Product decisions

- Keep a calm three-column task-management layout: list, flow, raw conversation/details.
- Group conversations by project and show only human-created main conversations at the first level. Each main conversation can expand to its derived Fork, new-thread, and Subagent conversations.
- Selecting any conversation defaults the center column to its message flow. Only a main conversation offers a task-view switch; derived conversations are conversation-only.
- In a main conversation's task view, show its tasks as a compact horizontal selector above the task graph. The selector count must match the main conversation's task count.
- Do not show a conversation-level `已结束` badge in the left navigation; show provenance and task count on main rows, and title plus creation method on child rows.
- Provide Graph and Swimlane task views.
- Task states are only `进行中`, `待验收`, and `已完成`; completion is manually confirmed.
- Execution-context states are only `运行中` and `已结束`.
- Context types describe creation provenance only: `主对话`, `Fork`, `新对话`, and `Subagent`. Do not infer implementation, data, testing, review, or other roles.
- Edges show direction only. Do not infer or display semantic reasons such as dispatch, rejection, or rework.
- Hovering a node highlights its connected route and separately marks all episodes from the same thread.
- Only show verifiable operational facts in details: workspace, branch, commit state, and whether changes are merged into the main workspace.
- Every execution node must expose work mode (`独立 worktree` or `主工作区`), commit state (`已提交` or `未提交`), and synchronization to the main workspace (`已同步`, `未同步`, or `无需同步`) without requiring the inspector.
- Use one reusable message-list component for both the center conversation detail and the right task-node inspector.
- Favor dense desktop information layout: compact rows and controls, consistent baselines, and minimal decorative whitespace while preserving readability.
