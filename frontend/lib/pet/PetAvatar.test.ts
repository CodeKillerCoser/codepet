// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { tick } from "svelte";
import { createClassComponent } from "svelte/legacy";
import { invoke } from "@tauri-apps/api/core";
import PetAvatar from "./PetAvatar.svelte";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
let component: ReturnType<typeof createClassComponent> | undefined;
let reduced = false;
let motionChanged: (() => void) | undefined;
beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
  reduced = false;
  vi.stubGlobal("matchMedia", () => ({ get matches() { return reduced; }, addEventListener: (_: string, fn: () => void) => { motionChanged = fn; }, removeEventListener: vi.fn() }));
});
afterEach(() => {
  component?.$destroy(); component = undefined;
  vi.useRealTimers(); vi.unstubAllGlobals(); vi.resetAllMocks(); document.body.innerHTML = "";
});
async function render(props = {}) {
  component = createClassComponent({ component: PetAvatar, target: document.body, props });
  await tick(); await Promise.resolve(); await tick();
}
const frame = () => document.querySelector<HTMLElement>("[data-pet-frame]")!;

it("plays frames, honors reduced motion and stops timers on destruction", async () => {
  await render();
  expect(frame().dataset.column).toBe("0");
  await vi.advanceTimersByTimeAsync(281); await tick();
  expect(frame().dataset.column).toBe("1");
  reduced = true; motionChanged?.(); await tick();
  expect(frame().dataset.column).toBe("0");
  await vi.advanceTimersByTimeAsync(2000); await tick();
  expect(frame().dataset.column).toBe("0");
  reduced = false; motionChanged?.(); await tick();
  component!.$destroy(); component = undefined;
  expect(vi.getTimerCount()).toBe(0);
});

it("ignores late asset responses after a pet switch and recovers failed images", async () => {
  let finish!: (url: string) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await render({ kind: "codex-atlas", imagePath: "/a.webp" });
  component!.$set({ petId: "builtin:panda", kind: "builtin", imagePath: null });
  await tick(); await Promise.resolve(); await tick();
  finish("data:image/webp;base64,old"); await tick(); await Promise.resolve(); await tick();
  expect(document.querySelector("img")!.src).toContain("panda.webp");
  vi.mocked(invoke).mockRejectedValueOnce(new Error("missing"));
  component!.$set({ kind: "image", imagePath: "/missing.png" });
  await tick(); await Promise.resolve(); await tick();
  expect(document.querySelector("[data-asset-error]")).not.toBeNull();
  expect(document.querySelector("img")!.src).not.toContain("old");
});

it("returns to the latest task state after petting and supports button activation", async () => {
  await render({ interactive: true });
  document.querySelector("button")!.click(); await tick();
  expect(document.querySelector(".pet-avatar")!.getAttribute("data-action")).toBe("celebrate");
  component!.$set({ status: "running" }); await tick();
  await vi.advanceTimersByTimeAsync(850); await tick();
  expect(frame().dataset.row).toBe("7");
  component!.$set({ status: "waiting-approval" }); await tick();
  document.querySelector("button")!.click(); await tick();
  expect(frame().dataset.row).toBe("6");
});

it("does not animate library thumbnails", async () => {
  await render({ animated: false });
  expect(vi.getTimerCount()).toBe(0);
});
