import type { AgentId, TokenUsage, TokenUsageSummary } from "./types";

export type UsageRange = "24h" | "7d" | "30d" | "90d" | "1y";
export type UsageBucketSize = "30m" | "1h" | "5h" | "12h" | "24h";

export interface UsageChartBucket {
  bucketStart: string;
  total: TokenUsage;
  agents: Partial<Record<AgentId, TokenUsage>>;
}

export interface UsageProviderTotal {
  provider: AgentId;
  total: TokenUsage;
}

export interface UsageChartData {
  total: TokenUsage;
  byProvider: UsageProviderTotal[];
  buckets: UsageChartBucket[];
  maxTokens: number;
}

export interface UsageChartOptions {
  range: UsageRange;
  bucketSize: UsageBucketSize;
  now?: Date;
}

const RANGE_MS: Record<UsageRange, number> = {
  "24h": 24 * 60 * 60 * 1000,
  "7d": 7 * 24 * 60 * 60 * 1000,
  "30d": 30 * 24 * 60 * 60 * 1000,
  "90d": 90 * 24 * 60 * 60 * 1000,
  "1y": 365 * 24 * 60 * 60 * 1000,
};

const BUCKET_MS: Record<UsageBucketSize, number> = {
  "30m": 30 * 60 * 1000,
  "1h": 60 * 60 * 1000,
  "5h": 5 * 60 * 60 * 1000,
  "12h": 12 * 60 * 60 * 1000,
  "24h": 24 * 60 * 60 * 1000,
};

export function buildUsageChartData(summary: TokenUsageSummary | null, options: UsageChartOptions): UsageChartData {
  const nowMs = options.now?.getTime() ?? Date.now();
  const fromMs = nowMs - RANGE_MS[options.range];
  const bucketMs = BUCKET_MS[options.bucketSize];
  const total = emptyUsage();
  const byProvider = new Map<AgentId, TokenUsage>();
  const buckets = new Map<string, UsageChartBucket>();

  for (const sourceBucket of summary?.byBucket ?? []) {
    const sourceMs = Date.parse(sourceBucket.bucketStart);
    if (Number.isNaN(sourceMs) || sourceMs < fromMs || sourceMs > nowMs) {
      continue;
    }

    const bucketStart = formatUtcBucket(Math.floor(sourceMs / bucketMs) * bucketMs);
    const bucket = buckets.get(bucketStart) ?? {
      bucketStart,
      total: emptyUsage(),
      agents: {},
    };
    const providerUsage = bucket.agents[sourceBucket.provider] ?? emptyUsage();
    addUsage(providerUsage, sourceBucket.total);
    bucket.agents[sourceBucket.provider] = providerUsage;
    addUsage(bucket.total, sourceBucket.total);
    buckets.set(bucketStart, bucket);

    addUsage(total, sourceBucket.total);
    const providerTotal = byProvider.get(sourceBucket.provider) ?? emptyUsage();
    addUsage(providerTotal, sourceBucket.total);
    byProvider.set(sourceBucket.provider, providerTotal);
  }

  const sortedBuckets = Array.from(buckets.values()).sort((left, right) => left.bucketStart.localeCompare(right.bucketStart));

  return {
    total,
    byProvider: Array.from(byProvider.entries())
      .map(([provider, total]) => ({ provider, total }))
      .sort((left, right) => left.provider.localeCompare(right.provider)),
    buckets: sortedBuckets,
    maxTokens: Math.max(0, ...sortedBuckets.map((bucket) => bucket.total.totalTokens)),
  };
}

export function yAxisTicks(maxTokens: number): number[] {
  if (maxTokens <= 0) return [0];
  return [maxTokens, maxTokens * 0.75, maxTokens * 0.5, maxTokens * 0.25, 0].map((value) => Math.round(value));
}

function emptyUsage(): TokenUsage {
  return {
    inputTokens: 0,
    cachedInputTokens: 0,
    outputTokens: 0,
    reasoningOutputTokens: 0,
    cacheCreationInputTokens: 0,
    cacheReadInputTokens: 0,
    totalTokens: 0,
  };
}

function addUsage(total: TokenUsage, usage: TokenUsage) {
  total.inputTokens += usage.inputTokens;
  total.cachedInputTokens += usage.cachedInputTokens;
  total.outputTokens += usage.outputTokens;
  total.reasoningOutputTokens += usage.reasoningOutputTokens;
  total.cacheCreationInputTokens += usage.cacheCreationInputTokens;
  total.cacheReadInputTokens += usage.cacheReadInputTokens;
  total.totalTokens += usage.totalTokens;
}

function formatUtcBucket(ms: number) {
  return new Date(ms).toISOString().replace(".000Z", "+00:00");
}
