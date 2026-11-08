// @vitest-environment jsdom
import { afterEach, expect, test, vi } from "vitest";
import { mount, unmount, tick } from "svelte";
import PermissionSettings from "./PermissionSettings.svelte";
import { getSystemPermissionSettings, openFullDiskAccessSettings } from "./permissions";

vi.mock("./permissions", () => ({ getSystemPermissionSettings: vi.fn(), openFullDiskAccessSettings: vi.fn(), revealPermissionApp: vi.fn() }));
let component: ReturnType<typeof mount> | undefined;
afterEach(async () => { if (component) await unmount(component); component = undefined; document.body.innerHTML = ""; vi.resetAllMocks(); });
async function render(supported = true) {
  vi.mocked(getSystemPermissionSettings).mockResolvedValue({ fullDiskAccessSupported: supported, appPath: "/Volumes/Test/Code Pet.app" });
  component = mount(PermissionSettings, { target: document.body });
  await tick(); await Promise.resolve(); await tick();
}
const authorize = () => Array.from(document.querySelectorAll("button")).find(button => button.textContent?.includes("前往授权"))!;

test("opening settings does not claim authorization and prevents duplicate requests", async () => {
  let finish!: () => void;
  vi.mocked(openFullDiskAccessSettings).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  await render();
  const button = authorize();
  button.focus(); button.click(); await tick();
  expect(button.disabled).toBe(true);
  button.click();
  expect(openFullDiskAccessSettings).toHaveBeenCalledTimes(1);
  finish(); await tick(); await Promise.resolve(); await tick();
  expect(button.disabled).toBe(false);
  expect(document.activeElement).toBe(button);
  expect(document.querySelector('[role="status"]')?.textContent).toContain("请在那里开启");
  expect(document.body.textContent).not.toContain("已授权");
  expect(document.body.textContent).toContain("/Volumes/Test/Code Pet.app");
});

test("failed system-settings launch is visible and retryable", async () => {
  vi.mocked(openFullDiskAccessSettings).mockRejectedValueOnce(new Error("无法打开系统设置")).mockResolvedValueOnce();
  await render(); authorize().click(); await tick(); await Promise.resolve(); await tick();
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("无法打开系统设置");
  expect(authorize().disabled).toBe(false);
  authorize().click(); await tick(); await Promise.resolve(); await tick();
  expect(document.querySelector('[role="alert"]')).toBeNull();
  expect(openFullDiskAccessSettings).toHaveBeenCalledTimes(2);
});

test("other platforms do not expose a macOS authorization action", async () => {
  await render(false);
  expect(authorize()).toBeUndefined();
  expect(document.body.textContent).toContain("当前系统没有 macOS");
  expect(openFullDiskAccessSettings).not.toHaveBeenCalled();
});

test("an unavailable backend shows a retry action instead of an authorization state", async () => {
  vi.mocked(getSystemPermissionSettings).mockRejectedValueOnce(new Error("Unknown command"))
    .mockResolvedValueOnce({ fullDiskAccessSupported: true, appPath: null });
  component = mount(PermissionSettings, { target: document.body });
  await tick(); await Promise.resolve(); await tick();
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("最新版本");
  expect(authorize()).toBeUndefined();
  document.querySelector('button')!.click(); await tick(); await Promise.resolve(); await tick();
  expect(authorize()).toBeDefined();
  expect(document.body.textContent).toContain("当前不是已打包的 App");
  expect(document.querySelector('[role="alert"]')).toBeNull();
});
