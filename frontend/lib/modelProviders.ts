export interface ModelProvider {
  id: string;
  name: string;
  endpoint: string;
  embedding: boolean;
}
export const modelProviders: ModelProvider[] = [
  {
    id: "local",
    name: "本地 MLX",
    endpoint: "http://127.0.0.1:11234/v1",
    embedding: true,
  },
  {
    id: "openai",
    name: "OpenAI",
    endpoint: "https://api.openai.com/v1",
    embedding: true,
  },
  {
    id: "deepseek",
    name: "DeepSeek",
    endpoint: "https://api.deepseek.com",
    embedding: false,
  },
  {
    id: "bailian",
    name: "阿里云百炼",
    endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    embedding: true,
  },
  {
    id: "siliconflow",
    name: "硅基流动",
    endpoint: "https://api.siliconflow.cn/v1",
    embedding: true,
  },
  { id: "custom", name: "自定义 · OpenAI 兼容", endpoint: "", embedding: true },
];
export const sameEndpoint = (a: string, b: string) =>
  a.trim().replace(/\/+$/, "") === b.trim().replace(/\/+$/, "");
export const providerId = (endpoint: string) =>
  modelProviders.find(
    (provider) =>
      provider.id !== "custom" && sameEndpoint(endpoint, provider.endpoint),
  )?.id ?? "custom";
export const isLocalEndpoint = (endpoint: string) => {
  try {
    return ["localhost", "127.0.0.1", "[::1]"].includes(
      new URL(endpoint).hostname,
    );
  } catch {
    return false;
  }
};
