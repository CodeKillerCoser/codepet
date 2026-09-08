import { expect, it } from "vitest";
import { makeUsageQuery } from "./usageQuery";
import type { UsageDataset } from "../../sdk/typescript/codepet-agent-sdk/src/generated";
const dataset: UsageDataset = {id: "observed-model-tokens", displayName: "Observed", scope: "local", metrics: ["totalTokens", "inputTokens", "outputTokens"], timeBuckets: ["halfHour", "day"], timeZones: [], modelFilter: true, modelGrouping: true, baseBucketMinutes: 30};
it("aligns range to half hours and requests whole-range summaries", () => {
  const q = makeUsageQuery(dataset, 1, "halfHour", " model-a ", true, Date.parse("2026-09-09T10:29:59Z"));
  expect(q.filter.time).toEqual({kind: "range", range: {from: "2026-09-08T10:00:00.000Z", to: "2026-09-09T10:30:00.000Z"}});
  expect(q.filter.modelIds).toEqual(["model-a"]);
  expect(q.aggregation.groupBy).toEqual(["model"]);
  expect(q.summaries).toEqual(["totals", "peakDaily"]);
});
it("respects daily-only capability without inventing a model breakdown", () => {
  const daily: UsageDataset = {...dataset, baseBucketMinutes: 1440, timeBuckets: ["day"], metrics: ["totalTokens"], modelFilter: false, modelGrouping: false};
  const q = makeUsageQuery(daily, 7, "day", "model-a", true, Date.parse("2026-09-09T10:30:00Z"));
  expect(q.filter.modelIds).toBeUndefined();
  expect(q.aggregation.groupBy).toEqual([]);
  expect(q.metrics).toEqual(["totalTokens"]);
  expect(q.filter.time).toEqual({kind: "range", range: {from: "2026-09-02T00:00:00.000Z", to: "2026-09-10T00:00:00.000Z"}});
  expect(() => makeUsageQuery(daily, 7, "halfHour", "", false)).toThrow();
});
