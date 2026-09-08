import type { UsageDataset, UsageQuery, UsageTimeBucket } from "../../sdk/typescript/codepet-agent-sdk/src/generated";

export const bucketLabels: Record<UsageTimeBucket, string> = { none: "累计", halfHour: "30 分钟", hour: "1 小时", day: "1 天", month: "1 个月" };
export function makeUsageQuery(dataset: UsageDataset, days: number, bucket: UsageTimeBucket, model: string, groupModels: boolean, now = Date.now()): UsageQuery {
  if (!dataset.timeBuckets.includes(bucket)) throw new Error("数据集不支持此时间单位");
  const precision = (dataset.baseBucketMinutes ?? 30) * 60_000;
  const to = Math.floor(now / precision) * precision + precision;
  const from = Math.floor((now - days * 86_400_000) / precision) * precision;
  return {
    datasetId: dataset.id,
    filter: { time: { kind: "range", range: { from: new Date(from).toISOString(), to: new Date(to).toISOString() } }, ...(dataset.modelFilter && model.trim() ? { modelIds: [model.trim()] } : {}) },
    aggregation: { timeBucket: bucket, timeZone: "UTC", groupBy: dataset.modelGrouping && groupModels ? ["model"] : [] },
    metrics: dataset.metrics,
    summaries: ["totals", ...(dataset.metrics.includes("totalTokens") ? ["peakDaily" as const] : [])],
    ...(bucket !== "none" ? { orderBy: [{ field: "bucketStart" as const, direction: "asc" as const }] } : {}),
    page: { limit: 200 },
  };
}
