// @vitest-environment jsdom
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { mount, unmount, tick } from "svelte";
import LocalModelEnvironment from "./LocalModelEnvironment.svelte";
import { runtimeRequest, type RuntimeStatus } from "./localModelRuntime";
vi.mock("./localModelRuntime", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./localModelRuntime")>()),
  runtimeRequest: vi.fn(),
}));
let component: ReturnType<typeof mount> | undefined;
const absent = (): RuntimeStatus => ({
  supported: true,
  reason: null,
  installation: null,
  endpoint: "http://127.0.0.1:11234/v1",
  job: null,
});
async function settle() {
  await tick();
  await Promise.resolve();
  await tick();
}
const button = (text: string) =>
  [...document.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === text,
  )!;
async function render() {
  const target = document.createElement("div");
  target.className = "main-theme";
  document.body.append(target);
  component = mount(LocalModelEnvironment, {
    target,
    props: { onUseLocal: vi.fn() },
  });
  await settle();
}
beforeEach(() => {
  vi.mocked(runtimeRequest).mockResolvedValue(absent());
});
afterEach(async () => {
  if (component) await unmount(component);
  component = undefined;
  document.body.innerHTML = "";
  vi.resetAllMocks();
});
test("existing system MLX is reused and never offers duplicate installation", async () => {
  vi.mocked(runtimeRequest).mockResolvedValue({
    ...absent(),
    installation: {
      name: "MLX Core",
      version: "26.9.2",
      path: "/Applications/MLX Core.app",
      source: "system",
    },
  });
  await render();
  expect(document.body.textContent).toContain("使用系统已有应用");
  expect(button("安装 MLX")).toBeUndefined();
  expect(button("打开 MLX")).toBeDefined();
});
test("install requires confirmation, exposes actual byte progress, and can be cancelled", async () => {
  let status = absent();
  vi.mocked(runtimeRequest).mockImplementation(
    async (action = "status", id) => {
      if (action === "prepare")
        return {
          id: "plan",
          version: "v26.11.3",
          bytes: 122812556,
          sourceUrl: "https://github.com/ddalcu/mlx-serve/releases",
        };
      if (action === "install") {
        expect(id).toBe("plan");
        status = {
          ...status,
          job: {
            id: "job",
            state: "downloading",
            stage: "正在下载 MLX-Serve",
            downloadedBytes: 31457280,
            totalBytes: 122812556,
            version: "v26.11.3",
            canCancel: true,
            error: null,
          },
        };
      }
      if (action === "cancel") {
        expect(id).toBe("job");
        status = {
          ...status,
          job: { ...status.job!, state: "cancelled", canCancel: false },
        };
      }
      return status;
    },
  );
  await render();
  button("安装 MLX").focus();
  button("安装 MLX").click();
  await settle();
  expect(document.querySelector("[role=dialog]")?.textContent).toContain(
    "不包含模型权重",
  );
  expect(
    vi.mocked(runtimeRequest).mock.calls.map((call) => call[0]),
  ).not.toContain("install");
  button("确认安装").click();
  await settle();
  await settle();
  expect(document.body.textContent).toContain("30.0 MB / 117.1 MB");
  expect(button("取消安装")).toBeDefined();
  button("取消安装").click();
  await settle();
  expect(document.body.textContent).toContain("已取消安装");
});
test("dismissing confirmation never starts installation", async () => {
  vi.mocked(runtimeRequest).mockImplementation(async (action) =>
    action === "prepare"
      ? {
          id: "plan",
          version: "v1",
          bytes: 100,
          sourceUrl: "https://github.com/ddalcu/mlx-serve/releases",
        }
      : absent(),
  );
  await render();
  button("安装 MLX").click();
  await settle();
  button("取消").click();
  await settle();
  expect(
    vi.mocked(runtimeRequest).mock.calls.map((call) => call[0]),
  ).not.toContain("install");
});
test("unsupported platform explains the limitation and has no install action", async () => {
  vi.mocked(runtimeRequest).mockResolvedValue({
    ...absent(),
    supported: false,
    reason: "需要 Apple Silicon Mac",
  });
  await render();
  expect(document.body.textContent).toContain("需要 Apple Silicon Mac");
  expect(button("安装 MLX")).toBeUndefined();
});

test("reopening the page resumes an active installation and allows cancellation", async () => {
  vi.mocked(runtimeRequest).mockResolvedValue({
    ...absent(),
    job: {
      id: "resumed",
      state: "downloading",
      stage: "正在下载 MLX-Serve",
      downloadedBytes: 20,
      totalBytes: 100,
      version: "v1",
      canCancel: true,
      error: null,
    },
  });
  await render();
  expect(document.querySelector("progress")?.value).toBe(20);
  expect(button("取消安装")).toBeDefined();
  expect(button("安装 MLX")).toBeUndefined();
});
test("failed metadata fetch can be retried without starting a download", async () => {
  vi.mocked(runtimeRequest).mockRejectedValueOnce(
    new Error("无法获取安装信息"),
  );
  await render();
  expect(document.body.textContent).toContain("无法获取安装信息");
  (
    document.querySelector(
      '[aria-label="重新检测本地环境"]',
    ) as HTMLButtonElement
  ).click();
  await settle();
  expect(button("安装 MLX")).toBeDefined();
  expect(
    vi.mocked(runtimeRequest).mock.calls.map((call) => call[0]),
  ).not.toContain("install");
});
