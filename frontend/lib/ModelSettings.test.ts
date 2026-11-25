// @vitest-environment jsdom
import { afterEach, expect, test, vi } from "vitest";
import { mount, unmount, tick } from "svelte";
import ModelSettings from "./ModelSettings.svelte";
import { modelGatewayRequest, modelCatalogRequest } from "./modelGateway";
vi.mock("./modelGateway", () => ({
  modelGatewayRequest: vi.fn(),
  modelCatalogRequest: vi.fn(async () => []),
}));
vi.mock("./localModelRuntime", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./localModelRuntime")>()),
  runtimeRequest: vi.fn(async () => ({
    supported: true,
    installation: {
      name: "MLX Core",
      version: "26.9.2",
      path: "/Applications/MLX Core.app",
      source: "system",
    },
    endpoint: "http://127.0.0.1:11234/v1",
    job: null,
  })),
}));
let component: ReturnType<typeof mount> | undefined;
const fixture = () => ({
  settings: {
    enabled: true,
    plainJson: false,
    llmExtraBody: {},
    llm: {
      endpoint: "http://127.0.0.1:11234/v1",
      model: "Qwen",
      apiKey: "",
      hasApiKey: true,
      dimensions: 0,
    },
    embedding: {
      endpoint: "http://127.0.0.1:11234/v1",
      model: "Embedding",
      apiKey: "",
      hasApiKey: false,
      dimensions: 1024,
    },
  },
  usage: {
    models: [
      {
        purpose: "llm",
        model: "Qwen",
        requests: 3,
        succeeded: 2,
        inputTokens: 100,
        outputTokens: 50,
        reportedRequests: 2,
      },
    ],
  },
});
afterEach(async () => {
  if (component) await unmount(component);
  component = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});
async function render() {
  vi.mocked(modelGatewayRequest).mockResolvedValue(fixture());
  component = mount(ModelSettings, { target: document.body });
  await tick();
  await Promise.resolve();
  await tick();
}
test("loads redacted credentials and distinguishes missing usage", async () => {
  await render();
  expect(document.querySelectorAll("input[type=password]")).toHaveLength(2);
  expect(
    (document.querySelector("input[type=password]") as HTMLInputElement).value,
  ).toBe("");
  expect(document.body.textContent).toContain("1 次未报告");
  expect(document.body.textContent).toContain("1 次失败");
});
test("saving prevents duplicate calls and recovers from errors", async () => {
  await render();
  let fail!: (reason: unknown) => void;
  vi.mocked(modelGatewayRequest).mockImplementationOnce(
    () =>
      new Promise((_, reject) => {
        fail = reject;
      }),
  );
  const button = document.querySelector(
    "button[type=submit]",
  ) as HTMLButtonElement;
  button.focus();
  button.click();
  await tick();
  expect(
    button.disabled ||
      (button.closest("fieldset") as HTMLFieldSetElement).disabled,
  ).toBe(true);
  button.click();
  expect(modelGatewayRequest).toHaveBeenCalledTimes(2);
  fail(new Error("模型地址无效"));
  await Promise.resolve();
  await tick();
  expect(
    document.querySelector(".model-config > [role=alert]")?.textContent,
  ).toContain("模型地址无效");
  expect((button.closest("fieldset") as HTMLFieldSetElement).disabled).toBe(
    false,
  );
  button.click();
  await tick();
  await Promise.resolve();
  await tick();
  expect(document.querySelector(".model-message")?.textContent).toContain(
    "已保存",
  );
});
test("invalid advanced JSON stays local and preserves the form", async () => {
  await render();
  const textarea = document.querySelector("textarea")!;
  textarea.value = "[1]";
  textarea.dispatchEvent(new Event("input", { bubbles: true }));
  (document.querySelector("button[type=submit]") as HTMLButtonElement).click();
  await tick();
  expect(
    document.querySelector(".model-config > [role=alert]")?.textContent,
  ).toContain("JSON 对象");
  expect(modelGatewayRequest).toHaveBeenCalledTimes(1);
});

test("usage refresh preserves unsaved model edits", async () => {
  await render();
  const input = document.querySelector(
    '[aria-label="llm 模型名称"]',
  ) as HTMLInputElement;
  input.value = "unsaved-model";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await tick();
  (document.querySelector(".model-usage") as HTMLDetailsElement).open = true;
  (
    Array.from(document.querySelectorAll("button")).find(
      (button) => button.textContent === "刷新用量",
    ) as HTMLButtonElement
  ).click();
  await tick();
  await Promise.resolve();
  await tick();
  expect(input.value).toBe("unsaved-model");
});

test("disabled processing can save incomplete model configuration", async () => {
  const result = fixture();
  result.settings.enabled = false;
  result.settings.embedding.dimensions = 0;
  vi.mocked(modelGatewayRequest).mockResolvedValue(result);
  component = mount(ModelSettings, { target: document.body });
  await tick();
  await Promise.resolve();
  await tick();
  (document.querySelector("button[type=submit]") as HTMLButtonElement).click();
  await tick();
  await Promise.resolve();
  await tick();
  expect(modelGatewayRequest).toHaveBeenCalledWith(
    "save-model-settings",
    expect.objectContaining({ enabled: false }),
  );
});
