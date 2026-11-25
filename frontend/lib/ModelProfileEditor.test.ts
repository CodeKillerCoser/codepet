// @vitest-environment jsdom
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { mount, unmount, tick } from "svelte";
import ModelProfileEditor from "./ModelProfileEditor.svelte";
import {
  modelCatalogRequest,
  type AvailableModel,
  type ModelProfile,
} from "./modelGateway";
vi.mock("./modelGateway", () => ({ modelCatalogRequest: vi.fn() }));
let component: ReturnType<typeof mount> | undefined;
const profile = (): ModelProfile => ({
  endpoint: "http://127.0.0.1:11234/v1",
  model: "configured",
  apiKey: "",
  hasApiKey: false,
  dimensions: 1024,
});
async function settle() {
  await tick();
  await Promise.resolve();
  await tick();
}
async function render(purpose: "llm" | "embedding" = "llm", draft = profile()) {
  component = mount(ModelProfileEditor, {
    target: document.body,
    props: {
      purpose,
      profile: draft,
      savedProfile: { ...draft },
      enabled: true,
    },
  });
  await settle();
}
function change(element: HTMLInputElement | HTMLSelectElement, value: string) {
  element.value = value;
  element.dispatchEvent(new Event("input", { bubbles: true }));
  element.dispatchEvent(new Event("change", { bubbles: true }));
}
beforeEach(() => {
  vi.mocked(modelCatalogRequest).mockResolvedValue([
    { id: "chat", kind: "llm" },
    { id: "vector", kind: "embedding" },
    { id: "voice", kind: "audio" },
  ]);
});
afterEach(async () => {
  if (component) await unmount(component);
  component = undefined;
  document.body.innerHTML = "";
  vi.resetAllMocks();
});
test("auto detects local models and filters known non-language models", async () => {
  await render();
  expect(modelCatalogRequest).toHaveBeenCalledTimes(1);
  const select = document.querySelector("#llm-model") as HTMLSelectElement;
  expect([...select.options].map((option) => option.value)).toEqual([
    "",
    "configured",
    "chat",
    "__manual_model__",
  ]);
  change(select, "__manual_model__");
  await settle();
  expect(document.activeElement).toBe(document.querySelector("#llm-model"));
});
test("cloud provider switch drops credentials and waits for a new key before discovery", async () => {
  await render("llm", {
    ...profile(),
    endpoint: "https://api.openai.com/v1",
    apiKey: "private",
    hasApiKey: true,
  });
  change(
    document.querySelector('[aria-label="语言模型服务提供方"]')!,
    "deepseek",
  );
  await settle();
  const key = document.querySelector("#llm-key") as HTMLInputElement;
  expect(key.value).toBe("");
  expect((document.querySelector("#llm-model") as HTMLInputElement).value).toBe(
    "",
  );
  expect(modelCatalogRequest).toHaveBeenCalledTimes(1);
  change(key, "new-key");
  await settle();
  expect(modelCatalogRequest).toHaveBeenCalledTimes(2);
  expect(vi.mocked(modelCatalogRequest).mock.calls[1][1].apiKey).toBe(
    "new-key",
  );
});
test("a late response from the prior endpoint cannot replace new model choices", async () => {
  let resolve!: (models: AvailableModel[]) => void;
  vi.mocked(modelCatalogRequest).mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  await render();
  change(
    document.querySelector('[aria-label="llm API 地址"]')!,
    "http://localhost:11434/v1",
  );
  await settle();
  resolve([{ id: "stale", kind: "llm" }]);
  await settle();
  expect(document.querySelector("#llm-model")?.textContent).toContain("chat");
  expect(document.body.textContent).not.toContain("stale");
});
test("manual fallback keeps configured model and does not guess embedding dimensions", async () => {
  vi.mocked(modelCatalogRequest).mockRejectedValueOnce(
    new Error("服务不提供模型列表"),
  );
  await render("embedding");
  expect(
    (document.querySelector("#embedding-model") as HTMLInputElement).value,
  ).toBe("configured");
  change(document.querySelector("#embedding-model")!, "new-vector");
  await settle();
  expect(
    (document.querySelector('[aria-label="向量维度"]') as HTMLInputElement)
      .value,
  ).toBe("0");
  expect((document.querySelector("details") as HTMLDetailsElement).open).toBe(
    true,
  );
});
