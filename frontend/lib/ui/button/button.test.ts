// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { mount, tick, unmount } from "svelte";
import BoundControlsQa from "../../../qa/BoundControlsQa.svelte";

let component: ReturnType<typeof mount> | undefined;
afterEach(async () => {
  if (component) await unmount(component);
  component = undefined;
  document.body.innerHTML = "";
});

describe("native UI control reference lifecycle", () => {
  it.each([false, true])("accepts unset refs and can mount again (initial null: %s)", async (initializeWithNull) => {
    component = mount(BoundControlsQa, { target: document.body, props: { initializeWithNull } });
    await tick();
    const toggle = document.querySelector<HTMLButtonElement>("button")!;
    for (let cycle = 0; cycle < 3; cycle += 1) {
      expect(document.querySelector("output")?.textContent).toBe("BUTTON/INPUT/TEXTAREA");
      toggle.click();
      await tick();
      expect(document.querySelector("output")?.textContent).toBe("none/none/none");
      toggle.click();
      await tick();
    }
    expect(document.querySelector("output")?.textContent).toBe("BUTTON/INPUT/TEXTAREA");
  });
});
