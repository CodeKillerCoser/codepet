import { describe, expect, it } from "vitest";
import { buildUsageChartData } from "./usageChart";
import type { TokenUsageSummary } from "./types";

function usage(inputTokens: number, outputTokens: number) {
  return {
    inputTokens,
    cachedInputTokens: 0,
    outputTokens,
    reasoningOutputTokens: 0,
    cacheCreationInputTokens: 0,
    cacheReadInputTokens: 0,
    totalTokens: inputTokens + outputTokens,
  };
}

function summary(): TokenUsageSummary {
  return {
    total: usage(0, 0),
    byProvider: [],
    byDay: [],
    byBucket: [
      { provider: "codex", bucketStart: "2026-05-27T08:00:00+00:00", sessions: 1, total: usage(100, 20) },
      { provider: "claude", bucketStart: "2026-05-27T08:30:00+00:00", sessions: 1, total: usage(40, 10) },
      { provider: "codex", bucketStart: "2026-05-27T09:00:00+00:00", sessions: 1, total: usage(50, 5) },
      { provider: "qoder", bucketStart: "2025-07-01T00:00:00+00:00", sessions: 1, total: usage(7, 3) },
      { provider: "qoder", bucketStart: "2024-05-27T00:00:00+00:00", sessions: 1, total: usage(200, 1) },
    ],
    sessions: [],
  };
}

describe("buildUsageChartData", () => {
  it("filters total usage by selected range", () => {
    const data = buildUsageChartData(summary(), {
      range: "1y",
      bucketSize: "30m",
      now: new Date("2026-05-28T00:00:00+00:00"),
    });

    expect(data.total.totalTokens).toBe(235);
    expect(data.byProvider.map((provider) => [provider.provider, provider.total.totalTokens])).toEqual([
      ["claude", 50],
      ["codex", 175],
      ["qoder", 10],
    ]);
  });

  it("regroups 30 minute source buckets into the selected unit", () => {
    const data = buildUsageChartData(summary(), {
      range: "24h",
      bucketSize: "1h",
      now: new Date("2026-05-28T00:00:00+00:00"),
    });

    expect(data.buckets.map((bucket) => [bucket.bucketStart, bucket.total.totalTokens])).toEqual([
      ["2026-05-27T08:00:00+00:00", 170],
      ["2026-05-27T09:00:00+00:00", 55],
    ]);
    expect(data.buckets[0].agents.codex?.totalTokens).toBe(120);
    expect(data.buckets[0].agents.claude?.totalTokens).toBe(50);
  });
});
