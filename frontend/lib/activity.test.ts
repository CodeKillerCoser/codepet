import { describe, expect, it } from "vitest";
import { activeActivities, activityCapabilities, activityKey, cardAgentLabel, cardEndTime, cardMessage, cardMeta, cardTitle, statusLabel, updateActivityList } from "./activity";
import type { ActivityFilterSettings, PetEvent } from "./types";

function event(overrides: Partial<PetEvent>): PetEvent {
  return {
    id: overrides.id ?? crypto.randomUUID(),
    provider: overrides.provider ?? "codex",
    kind: overrides.kind ?? "task-started",
    status: overrides.status ?? "thinking",
    title: overrides.title ?? "任务开始",
    message: overrides.message ?? "test",
    sessionId: "sessionId" in overrides ? overrides.sessionId : "session-a",
    cwd: "cwd" in overrides ? overrides.cwd : "/workspace/a",
    toolName: overrides.toolName ?? null,
    shouldRing: overrides.shouldRing ?? false,
    createdAt: overrides.createdAt ?? "2026-05-26T06:00:00.000Z",
    raw: overrides.raw ?? {},
    source: overrides.source ?? null,
  };
}

function filters(overrides: Partial<ActivityFilterSettings>): ActivityFilterSettings {
  return {
    titleKeywords: overrides.titleKeywords ?? [],
    messageKeywords: overrides.messageKeywords ?? [],
    byAgent: overrides.byAgent ?? {},
  };
}

describe("activeActivities", () => {
  it("keeps one latest card per provider session and ignores orphan completed sessions", () => {
    const activities = activeActivities(
      [
        event({ id: "codex-1", provider: "codex", sessionId: "codex-a", message: "old", status: "thinking", createdAt: "2026-05-26T06:00:00.000Z" }),
        event({ id: "claude-1", provider: "claude", sessionId: "claude-a", message: "read file", status: "running", createdAt: "2026-05-26T06:01:00.000Z" }),
        event({ id: "codex-2", provider: "codex", sessionId: "codex-a", message: "new", status: "running", createdAt: "2026-05-26T06:02:00.000Z" }),
        event({ id: "qoder-1", provider: "qoder", sessionId: "qoder-a", message: "done", status: "done", createdAt: "2026-05-26T06:03:00.000Z" }),
      ],
      4,
      new Date("2026-05-26T06:03:00.000Z"),
    );

    expect(activities.map((activity) => activity.id)).toEqual(["claude-1", "codex-2"]);
    expect(activities.find((activity) => activity.provider === "codex")?.message).toBe("new");
  });

  it("returns no activities when every session is idle or only has an orphan done event", () => {
    const activities = activeActivities([
      event({ id: "codex-done", provider: "codex", sessionId: "codex-a", status: "done" }),
      event({ id: "claude-idle", provider: "claude", sessionId: "claude-a", status: "idle" }),
    ]);

    expect(activities).toEqual([]);
  });

  it("keeps a completed card when a listed session finishes", () => {
    const activities = activeActivities(
      [
        event({ id: "codex-start", provider: "codex", sessionId: "codex-a", message: "评审 agent token统计实现", status: "thinking", createdAt: "2026-05-26T06:00:00.000Z" }),
        event({ id: "codex-done", provider: "codex", sessionId: "codex-a", message: "已修复并跑完真实验证", status: "done", createdAt: "2026-05-26T06:02:00.000Z" }),
      ],
      4,
      new Date("2026-05-26T06:03:00.000Z"),
    );

    expect(activities.map((activity) => activity.id)).toEqual(["codex-done"]);
    expect(activities[0].status).toBe("done");
    expect(activities[0].title).toBe("评审 agent token统计实现");
    expect(activities[0].message).toBe("已修复并跑完真实验证");
  });

  it("does not create a pet task for Codex background sessions that only emit lifecycle events", () => {
    const activities = activeActivities(
      [
        event({
          id: "background-start",
          provider: "codex",
          sessionId: "background-session",
          title: "任务开始",
          message: "SessionStart",
          status: "thinking",
          createdAt: "2026-05-26T06:00:00.000Z",
        }),
        event({
          id: "background-done",
          provider: "codex",
          sessionId: "background-session",
          kind: "task-completed",
          title: "任务完成",
          message: "已完成增量 consolidation，主要更新了 [MEMORY.md]。",
          status: "done",
          createdAt: "2026-05-26T06:01:00.000Z",
        }),
      ],
      4,
      new Date("2026-05-26T06:02:00.000Z"),
    );

    expect(activities).toEqual([]);
  });

  it("does not create a pet task for Codex internal title-generation sessions", () => {
    const activities = activeActivities(
      [
        event({
          id: "title-start",
          provider: "codex",
          sessionId: "title-session",
          title: "任务开始",
          message: "SessionStart",
          status: "thinking",
          createdAt: "2026-05-26T06:00:00.000Z",
        }),
        event({
          id: "title-prompt",
          provider: "codex",
          sessionId: "title-session",
          title: "SessionStart",
          message: "You are a helpful assistant. You will be presented with a user prompt, and your job is to provide a short title for a task that will be created from that prompt.",
          status: "thinking",
          createdAt: "2026-05-26T06:00:01.000Z",
        }),
        event({
          id: "title-done",
          provider: "codex",
          sessionId: "title-session",
          kind: "task-completed",
          title: "SessionStart",
          message: "{\"title\":\"生成熊猫烧香4K壁纸\"}",
          status: "done",
          createdAt: "2026-05-26T06:00:08.000Z",
        }),
      ],
      4,
      new Date("2026-05-26T06:01:00.000Z"),
    );

    expect(activities).toEqual([]);
  });

  it("does not create a pet task for Codex hyperpersonalized suggestion sessions", () => {
    const activities = activeActivities(
      [
        event({
          id: "suggestion-start",
          provider: "codex",
          sessionId: "suggestion-session",
          title: "任务开始",
          message: "SessionStart",
          status: "thinking",
          createdAt: "2026-05-26T06:00:00.000Z",
        }),
        event({
          id: "suggestion-prompt",
          provider: "codex",
          sessionId: "suggestion-session",
          title: "SessionStart",
          message:
            "# Overview\n\nGenerate 0 to 3 hyperpersonalized suggestions for what this user can do with Codex in this local project: /Users/wangxin/Developer/Work/wukong-studio\n\nRecent Codex threads in this project:\n- 评审 agent token统计实现\n\n# Response format\nEach suggestion must include: title, description, prompt, appId",
          status: "thinking",
          createdAt: "2026-05-26T06:00:01.000Z",
        }),
        event({
          id: "suggestion-tool",
          provider: "codex",
          sessionId: "suggestion-session",
          kind: "tool-started",
          title: "SessionStart",
          message: "python3 - <<'PY'\nprint('scan')\nPY",
          status: "running",
          createdAt: "2026-05-26T06:00:02.000Z",
        }),
        event({
          id: "suggestion-done",
          provider: "codex",
          sessionId: "suggestion-session",
          kind: "task-completed",
          title: "SessionStart",
          message: "已完成增量 consolidation，主要更新了 [MEMORY.md]。",
          status: "done",
          createdAt: "2026-05-26T06:00:08.000Z",
        }),
      ],
      4,
      new Date("2026-05-26T06:01:00.000Z"),
    );

    expect(activities).toEqual([]);
  });

  it("drops stale thinking and running events but keeps attention states", () => {
    const activities = activeActivities(
      [
        event({ id: "old-running", provider: "claude", sessionId: "claude-a", status: "running", createdAt: "2026-05-26T06:00:00.000Z" }),
        event({ id: "old-approval", provider: "codex", sessionId: "codex-a", status: "waiting-approval", createdAt: "2026-05-26T06:00:00.000Z" }),
        event({ id: "fresh-running", provider: "qoder", sessionId: "qoder-a", status: "running", createdAt: "2026-05-26T06:25:00.000Z" }),
      ],
      4,
      new Date("2026-05-26T06:40:00.000Z"),
    );

    expect(activities.map((activity) => activity.id)).toEqual(["fresh-running", "old-approval"]);
  });

  it("keeps the prompt title when later tool events only carry transcript paths", () => {
    const activities = activeActivities(
      [
        event({ id: "prompt", sessionId: "codex-a", message: "排查桌宠刷新", status: "thinking", createdAt: "2026-05-26T06:00:00.000Z" }),
        event({ id: "tool", sessionId: "codex-a", message: "/Users/wangxin/.codex/sessions/2026/05/26/rollout.jsonl", status: "running", createdAt: "2026-05-26T06:01:00.000Z" }),
      ],
      4,
      new Date("2026-05-26T06:02:00.000Z"),
    );

    expect(activities[0].id).toBe("tool");
    expect(activities[0].status).toBe("running");
    expect(activities[0].message).toBe("排查桌宠刷新");
  });

  it("keeps the prompt title when later Windows tool events only carry transcript paths", () => {
    const activities = activeActivities(
      [
        event({ id: "prompt", sessionId: "codex-a", message: "继续修复乱码", status: "thinking", createdAt: "2026-05-26T06:00:00.000Z" }),
        event({
          id: "tool",
          sessionId: "codex-a",
          message: String.raw`\\?\E:\.codex\sessions\2026\06\06\rollout.jsonl`,
          status: "running",
          createdAt: "2026-05-26T06:01:00.000Z",
        }),
      ],
      4,
      new Date("2026-05-26T06:02:00.000Z"),
    );

    expect(activities[0].id).toBe("tool");
    expect(activities[0].title).toBe("继续修复乱码");
    expect(activities[0].message).toBe("继续修复乱码");
  });

  it("keeps the prompt title when Claude completion title is a transcript path", () => {
    const activities = activeActivities(
      [
        event({
          id: "prompt",
          provider: "claude",
          sessionId: "claude-a",
          title: "任务开始",
          message: "今天天气怎么样",
          status: "thinking",
          createdAt: "2026-05-26T06:00:00.000Z",
        }),
        event({
          id: "done",
          provider: "claude",
          sessionId: "claude-a",
          kind: "task-completed",
          title: "/Users/wangxin/.claude/projects/-Users-wangxin/session.jsonl",
          message: "我没有获取实时天气数据的能力。",
          status: "done",
          createdAt: "2026-05-26T06:01:00.000Z",
        }),
      ],
      4,
      new Date("2026-05-26T06:02:00.000Z"),
    );

    expect(activities[0].id).toBe("done");
    expect(activities[0].title).toBe("今天天气怎么样");
    expect(activities[0].message).toBe("我没有获取实时天气数据的能力。");
  });

  it("lets a later authoritative title replace an earlier prompt fallback", () => {
    const activities = activeActivities(
      [
        event({
          id: "prompt",
          sessionId: "codex-a",
          title: "任务开始",
          message: "先改观测台，任务状态按automation.status判定",
          status: "thinking",
          createdAt: "2026-05-26T06:00:00.000Z",
        }),
        event({
          id: "tool",
          sessionId: "codex-a",
          title: "评审 agent token统计实现",
          message: "工具：Bash",
          status: "running",
          createdAt: "2026-05-26T06:01:00.000Z",
        }),
      ],
      undefined,
      new Date("2026-05-26T06:02:00.000Z"),
    );

    expect(cardTitle(activities[0])).toBe("评审 agent token统计实现");
    expect(cardMessage(activities[0])).toBe("工具：Bash");
  });

  it("keeps more than four activities so the pet list can scroll", () => {
    const activities = activeActivities(
      Array.from({ length: 5 }, (_, index) =>
        event({
          id: `codex-${index}`,
          sessionId: `codex-${index}`,
          message: `task ${index}`,
          status: "running",
          createdAt: `2026-05-26T06:0${index}:00.000Z`,
        }),
      ),
      undefined,
      new Date("2026-05-26T06:05:00.000Z"),
    );

    expect(activities).toHaveLength(5);
    expect(activities.map((activity) => activity.id)).toEqual(["codex-4", "codex-3", "codex-2", "codex-1", "codex-0"]);
  });

  it("filters activities by custom title keywords", () => {
    const activities = activeActivities(
      [
        event({ id: "memory", title: "Codex memory summary", message: "writing memory", sessionId: "memory-session" }),
        event({ id: "real", title: "Review implementation", message: "inspect code", sessionId: "real-session" }),
      ],
      undefined,
      new Date("2026-05-26T06:02:00.000Z"),
      filters({ titleKeywords: ["memory summary"] }),
    );

    expect(activities.map((activity) => activity.id)).toEqual(["real"]);
  });

  it("filters activities by custom message keywords", () => {
    const activities = activeActivities(
      [
        event({ id: "suggestion", title: "SessionStart", message: "Recent Codex threads in this project", sessionId: "suggestion-session" }),
        event({ id: "real", title: "Build package", message: "npm run build", sessionId: "real-session" }),
      ],
      undefined,
      new Date("2026-05-26T06:02:00.000Z"),
      filters({ messageKeywords: ["codex threads"] }),
    );

    expect(activities.map((activity) => activity.id)).toEqual(["real"]);
  });

  it("applies custom filters only to the matching agent", () => {
    const activities = activeActivities(
      [
        event({ id: "codex-memory", provider: "codex", title: "Memory summary", message: "internal", sessionId: "codex-memory" }),
        event({ id: "claude-memory", provider: "claude", title: "Memory summary", message: "user task", sessionId: "claude-memory" }),
        event({ id: "codex-real", provider: "codex", title: "Build package", message: "npm run build", sessionId: "codex-real" }),
      ],
      undefined,
      new Date("2026-05-26T06:02:00.000Z"),
      filters({
        byAgent: {
          codex: { titleKeywords: ["memory"], messageKeywords: [] },
        },
      }),
    );

    expect(activities.map((activity) => activity.id)).toEqual(["claude-memory", "codex-real"]);
  });

  it("does not reorder an already active task when later hook updates arrive", () => {
    const activities = activeActivities(
      [
        event({ id: "codex-start", provider: "codex", sessionId: "codex-a", status: "thinking", createdAt: "2026-05-26T06:00:00.000Z" }),
        event({ id: "claude-start", provider: "claude", sessionId: "claude-a", status: "thinking", createdAt: "2026-05-26T06:05:00.000Z" }),
        event({ id: "codex-latest", provider: "codex", sessionId: "codex-a", status: "running", createdAt: "2026-05-26T06:10:00.000Z" }),
      ],
      undefined,
      new Date("2026-05-26T06:11:00.000Z"),
    );

    expect(activities.map((activity) => activity.id)).toEqual(["claude-start", "codex-latest"]);
  });

  it("refreshes sort position when a completed listed task becomes active again", () => {
    const activities = activeActivities(
      [
        event({ id: "codex-start", provider: "codex", sessionId: "codex-a", status: "thinking", createdAt: "2026-05-26T06:00:00.000Z" }),
        event({ id: "codex-done", provider: "codex", sessionId: "codex-a", status: "done", createdAt: "2026-05-26T06:02:00.000Z" }),
        event({ id: "claude-start", provider: "claude", sessionId: "claude-a", status: "thinking", createdAt: "2026-05-26T06:05:00.000Z" }),
        event({ id: "codex-restarted", provider: "codex", sessionId: "codex-a", status: "running", createdAt: "2026-05-26T06:10:00.000Z" }),
      ],
      undefined,
      new Date("2026-05-26T06:11:00.000Z"),
    );

    expect(activities.map((activity) => activity.id)).toEqual(["codex-restarted", "claude-start"]);
  });
});

describe("updateActivityList", () => {
  it("updates the current display list incrementally instead of rebuilding from historical events", () => {
    const dismissedKeys = new Set<string>();
    const started = event({ id: "codex-start", sessionId: "codex-a", status: "thinking", message: "开始任务" });
    const completed = event({ id: "codex-done", sessionId: "codex-a", status: "done", message: "任务完成", createdAt: "2026-05-26T06:02:00.000Z" });
    const orphanDone = event({ id: "orphan-done", sessionId: "codex-b", status: "done", message: "旧任务完成", createdAt: "2026-05-26T06:03:00.000Z" });

    const active = updateActivityList([], [started], dismissedKeys, new Date("2026-05-26T06:01:00.000Z"));
    const finished = updateActivityList(active, [completed], dismissedKeys, new Date("2026-05-26T06:03:00.000Z"));
    const unchanged = updateActivityList(finished, [orphanDone], dismissedKeys, new Date("2026-05-26T06:04:00.000Z"));

    expect(unchanged.map((activity) => activity.id)).toEqual(["codex-done"]);
    expect(unchanged[0].title).toBe("开始任务");
  });

  it("keeps the terminal event time when a listed task finishes", () => {
    const started = event({
      id: "codex-start",
      sessionId: "codex-a",
      status: "thinking",
      message: "开始任务",
      createdAt: "2026-05-26T06:00:00.000Z",
    });
    const completed = event({
      id: "codex-done",
      sessionId: "codex-a",
      status: "done",
      message: "任务完成",
      createdAt: "2026-05-26T06:02:00.000Z",
    });

    const active = updateActivityList([], [started], new Set<string>(), new Date("2026-05-26T06:01:00.000Z"));
    const [finished] = updateActivityList(active, [completed], new Set<string>(), new Date("2026-05-26T06:03:00.000Z"));

    const expectedTime = new Intl.DateTimeFormat(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    }).format(new Date("2026-05-26T06:02:00.000Z"));
    expect(finished.createdAt).toBe("2026-05-26T06:00:00.000Z");
    expect(finished.endedAt).toBe("2026-05-26T06:02:00.000Z");
    expect(cardEndTime(finished)).toBe(expectedTime);
    expect(cardMeta(finished)).toBe(`codex · 任务完成 · ${expectedTime}`);
  });

  it("matches a keyless completed event to the latest active task for the same provider", () => {
    const started = event({
      id: "codex-start",
      provider: "codex",
      sessionId: "codex-a",
      cwd: "/workspace/a",
      status: "running",
      message: "Windows 任务",
      createdAt: "2026-05-26T06:00:00.000Z",
    });
    const completed = event({
      id: "codex-done",
      provider: "codex",
      sessionId: null,
      cwd: null,
      status: "done",
      message: "Stop",
      createdAt: "2026-05-26T06:02:00.000Z",
    });

    const active = updateActivityList([], [started], new Set<string>(), new Date("2026-05-26T06:01:00.000Z"));
    const [finished] = updateActivityList(active, [completed], new Set<string>(), new Date("2026-05-26T06:03:00.000Z"));

    expect(finished.id).toBe("codex-done");
    expect(finished.status).toBe("done");
    expect(finished.sessionId).toBe("codex-a");
    expect(finished.cwd).toBe("/workspace/a");
    expect(finished.title).toBe("Windows 任务");
  });

  it("keeps dismissed completed tasks out until the same task becomes active again", () => {
    const started = event({ id: "codex-start", sessionId: "codex-a", status: "thinking", message: "开始任务" });
    const completed = event({ id: "codex-done", sessionId: "codex-a", status: "done", message: "任务完成" });
    const dismissedKeys = new Set([activityKey(started)]);

    const afterDismissedDone = updateActivityList([], [completed], dismissedKeys, new Date("2026-05-26T06:01:00.000Z"));
    const restarted = updateActivityList(
      afterDismissedDone,
      [event({ id: "codex-restart", sessionId: "codex-a", status: "running", message: "重新执行" })],
      dismissedKeys,
      new Date("2026-05-26T06:01:00.000Z"),
    );

    expect(afterDismissedDone).toEqual([]);
    expect(restarted.map((activity) => activity.id)).toEqual(["codex-restart"]);
    expect(dismissedKeys.has(activityKey(started))).toBe(false);
  });

  it("keeps the known terminal source when later events omit source context", () => {
    const started = event({
      id: "codex-cli-start",
      provider: "codex",
      sessionId: "codex-a",
      status: "thinking",
      message: "开始任务",
      source: {
        terminalProgram: "Apple_Terminal",
        ttyPath: "/dev/ttys018",
        appBundleId: "com.apple.Terminal",
      },
    });
    const updatedWithoutSource = event({
      id: "codex-cli-update",
      provider: "codex",
      sessionId: "codex-a",
      status: "running",
      message: "继续执行",
      source: null,
    });

    const [activity] = updateActivityList(
      updateActivityList([], [started], new Set<string>(), new Date("2026-05-26T06:00:00.000Z")),
      [updatedWithoutSource],
      new Set<string>(),
      new Date("2026-05-26T06:01:00.000Z"),
    );

    expect(cardAgentLabel(activity)).toBe("codex cli");
    expect(cardMeta(activity)).toBe("codex cli · 正在执行");
  });

  it("keeps Codex internal sessions hidden across incremental event batches", () => {
    const hiddenInternalKeys = new Set<string>();
    const prompt = event({
      id: "suggestion-prompt",
      provider: "codex",
      sessionId: "suggestion-session",
      title: "SessionStart",
      message:
        "# Overview\n\nGenerate 0 to 3 hyperpersonalized suggestions for what this user can do with Codex in this local project: /Users/wangxin/Developer/Work/wukong-studio",
      status: "thinking",
      createdAt: "2026-05-26T06:00:01.000Z",
    });
    const tool = event({
      id: "suggestion-tool",
      provider: "codex",
      sessionId: "suggestion-session",
      kind: "tool-started",
      title: "SessionStart",
      message: "python3 - <<'PY'\nprint('scan')\nPY",
      status: "running",
      createdAt: "2026-05-26T06:00:02.000Z",
    });

    const afterPrompt = updateActivityList([], [prompt], new Set<string>(), new Date("2026-05-26T06:00:01.000Z"), hiddenInternalKeys);
    const afterTool = updateActivityList(afterPrompt, [tool], new Set<string>(), new Date("2026-05-26T06:00:02.000Z"), hiddenInternalKeys);

    expect(afterPrompt).toEqual([]);
    expect(afterTool).toEqual([]);
    expect(hiddenInternalKeys.has(activityKey(prompt))).toBe(true);
  });

  it("keeps custom-filtered sessions hidden across incremental event batches", () => {
    const hiddenInternalKeys = new Set<string>();
    const prompt = event({
      id: "memory-prompt",
      provider: "codex",
      sessionId: "memory-session",
      title: "Memory summary",
      message: "start",
      status: "thinking",
      createdAt: "2026-05-26T06:00:01.000Z",
    });
    const tool = event({
      id: "memory-tool",
      provider: "codex",
      sessionId: "memory-session",
      kind: "tool-started",
      title: "Bash",
      message: "cat MEMORY.md",
      status: "running",
      createdAt: "2026-05-26T06:00:02.000Z",
    });

    const afterPrompt = updateActivityList([], [prompt], new Set<string>(), new Date("2026-05-26T06:00:01.000Z"), hiddenInternalKeys, filters({ titleKeywords: ["memory"] }));
    const afterTool = updateActivityList(afterPrompt, [tool], new Set<string>(), new Date("2026-05-26T06:00:02.000Z"), hiddenInternalKeys, filters({ titleKeywords: ["memory"] }));

    expect(afterPrompt).toEqual([]);
    expect(afterTool).toEqual([]);
    expect(hiddenInternalKeys.has(activityKey(prompt))).toBe(true);
  });
});

describe("statusLabel", () => {
  it("uses stable display labels for all active states", () => {
    expect(statusLabel("thinking")).toBe("正在思考");
    expect(statusLabel("running")).toBe("正在执行");
    expect(statusLabel("waiting-approval")).toBe("等待授权");
    expect(statusLabel("done")).toBe("任务完成");
  });
});

describe("card display", () => {
  it("renders title, latest message, and agent status as separate rows", () => {
    const activity = event({
      provider: "codex",
      status: "done",
      title: "评审 agent token统计实现",
      message: "已修复并跑完真实验证。关键修正是 TurnCollector 现在按真实 assistant...",
    });

    expect(cardTitle(activity)).toBe("评审 agent token统计实现");
    expect(cardMessage(activity)).toBe("已修复并跑完真实验证。关键修正是 TurnCollector 现在按真实 assistant...");
    expect(cardMeta(activity)).toBe(`codex · 任务完成 · ${cardEndTime(activity)}`);
  });

  it("uses the prompt message as the visible title while a generic task is active", () => {
    const activity = event({
      status: "thinking",
      title: "任务开始",
      message: "hello一下",
    });

    expect(cardTitle(activity)).toBe("hello一下");
    expect(cardMessage(activity)).toBe("hello一下");
  });

  it("does not use a terminal assistant summary as the visible title", () => {
    const activity = event({
      status: "done",
      title: "任务完成",
      message: "修好了。根因确认是 Collector 返回的 JSON 字节本身是 UTF-8 正常的。",
    });

    expect(cardTitle(activity)).toBe("任务完成");
    expect(cardMessage(activity)).toBe("修好了。根因确认是 Collector 返回的 JSON 字节本身是 UTF-8 正常的。");
  });

  it("does not render Windows transcript paths as card title or message", () => {
    const activity = event({
      status: "running",
      title: "正在执行工具",
      message: String.raw`\\?\E:\.codex\sessions\2026\06\06\rollout.jsonl`,
    });

    expect(cardTitle(activity)).toBe("正在执行工具");
    expect(cardMessage(activity)).toBe("正在执行");
  });

  it("marks terminal-sourced agent cards as cli in the footer", () => {
    const activity = event({
      provider: "cursor",
      status: "running",
      source: {
        terminalProgram: "Apple_Terminal",
        ttyPath: "/dev/ttys018",
        appBundleId: "com.apple.Terminal",
      },
    });

    expect(cardMeta(activity)).toBe("cursor cli · 正在执行");
  });

  it("distinguishes Codex and Qoder app versus cli sources", () => {
    expect(cardAgentLabel(event({
      provider: "codex",
      source: { appBundleId: "com.openai.codex" },
    }))).toBe("codex");
    expect(cardAgentLabel(event({
      provider: "codex",
      source: { terminalProgram: "Apple_Terminal", ttyPath: "/dev/ttys018", appBundleId: "com.apple.Terminal" },
    }))).toBe("codex cli");
    expect(cardAgentLabel(event({
      provider: "qoder",
      source: { appBundleId: "com.qoder.ide" },
    }))).toBe("qoder");
    expect(cardAgentLabel(event({
      provider: "qoder",
      source: { terminalProgram: "iTerm.app", ttyPath: "/dev/ttys007", appBundleId: "com.googlecode.iterm2" },
    }))).toBe("qoder cli");
  });

  it("keeps app-sourced agent cards using the current provider footer", () => {
    const activity = event({
      provider: "codex",
      status: "running",
      source: {
        appBundleId: "com.openai.codex",
      },
    });

    expect(cardMeta(activity)).toBe("codex · 正在执行");
  });

  it("does not show end time for active cards", () => {
    const activity = event({
      provider: "codex",
      status: "running",
      createdAt: "2026-05-26T06:02:00.000Z",
    });

    expect(cardEndTime(activity)).toBe("");
    expect(cardMeta(activity)).toBe("codex · 正在执行");
  });
});

describe("activityCapabilities", () => {
  it("exposes reply for completed Codex Desktop events with a thread id", () => {
    const activity = event({
      provider: "codex",
      status: "done",
      sessionId: "019e66f1-4d9e-78e2-8f87-f07c0251ce36",
      source: {
        appBundleId: "com.openai.codex",
      },
    });

    expect(activityCapabilities(activity).canReply).toBe(true);
  });

  it("does not expose reply for running Codex events", () => {
    const activity = event({
      provider: "codex",
      status: "running",
      sessionId: "019e66f1-4d9e-78e2-8f87-f07c0251ce36",
    });

    expect(activityCapabilities(activity).canReply).toBe(false);
  });

  it("does not expose reply for Codex events without a thread id", () => {
    const activity = event({
      provider: "codex",
      sessionId: null,
    });

    expect(activityCapabilities(activity).canReply).toBe(false);
  });

  it("does not expose reply for Claude events because there is no reliable conversation reply protocol", () => {
    const activity = event({
      provider: "claude",
      source: {
        terminalProgram: "Apple_Terminal",
        ttyPath: "/dev/ttys018",
      },
    });

    expect(activityCapabilities(activity).canReply).toBe(false);
  });

  it("does not expose reply for Qoder remote-control sessions until an existing-session send API is verified", () => {
    const remoteActivity = event({
      provider: "qoder",
      status: "done",
      sessionId: "qoder-session",
    });
    const missingSessionActivity = event({
      provider: "qoder",
      status: "done",
      sessionId: null,
    });

    expect(activityCapabilities(remoteActivity).canReply).toBe(false);
    expect(activityCapabilities(missingSessionActivity).canReply).toBe(false);
  });

  it("does not expose reply for running Qoder remote-control sessions", () => {
    const activity = event({
      provider: "qoder",
      status: "running",
      sessionId: "qoder-session",
    });

    expect(activityCapabilities(activity).canReply).toBe(false);
  });

  it("exposes approval controls only for active permission requests", () => {
    const approval = activityCapabilities(event({ provider: "qoder", status: "waiting-approval" }));
    expect(approval.canApprove).toBe(true);
    expect(approval.canReply).toBe(false);
    expect(activityCapabilities(event({ provider: "qoder", status: "running" })).canApprove).toBe(false);
  });

  it("exposes approval controls for Codex permission requests", () => {
    const activity = event({
      provider: "codex",
      status: "waiting-approval",
    });

    expect(activityCapabilities(activity).canApprove).toBe(true);
  });
});
