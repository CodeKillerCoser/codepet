import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { createWindowChrome, windowPlatform } from "./windowChrome";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: vi.fn() }));

describe("main window platform boundary", () => {
  const unlisten = vi.fn();
  const native = {setDecorations: vi.fn(), isMaximized: vi.fn(), onResized: vi.fn(), minimize: vi.fn(), toggleMaximize: vi.fn(), close: vi.fn()};
  beforeEach(() => {
    vi.resetAllMocks();
    vi.stubGlobal("navigator", {userAgent: "Windows NT 10.0"});
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(getCurrentWindow).mockReturnValue(native as never);
    native.isMaximized.mockResolvedValue(false);
    native.onResized.mockResolvedValue(unlisten);
  });
  afterEach(() => vi.unstubAllGlobals());
  it.each([["Macintosh; Intel Mac OS X", "macos"], ["Windows NT 10.0", "windows"], ["X11; Linux", "other"]])("classifies %s", (ua, platform) => {
    expect(windowPlatform(ua)).toBe(platform);
  });
  it("does not call native APIs in browser previews", async () => {
    vi.mocked(isTauri).mockReturnValue(false);
    const chrome = createWindowChrome();
    (await chrome.initialize(vi.fn(), vi.fn()))();
    await chrome.minimize(); await chrome.toggleMaximize(); await chrome.close();
    expect(getCurrentWindow).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });
  it("preserves native macOS decorations", async () => {
    vi.stubGlobal("navigator", {userAgent:"Macintosh"});
    await createWindowChrome().initialize(vi.fn(), vi.fn());
    expect(native.setDecorations).not.toHaveBeenCalled();
  });
  it("uses native macOS geometry at startup and after resizing", async () => {
    vi.stubGlobal("navigator", {userAgent: "Macintosh"});
    const anchor = vi.fn(); const error = vi.fn();
    vi.mocked(invoke).mockResolvedValueOnce({left: 83, centerY: 14});
    const cleanup = await createWindowChrome().initialize(vi.fn(), error, anchor);
    expect(invoke).toHaveBeenCalledWith("main_window_toolbar_anchor");
    expect(anchor).toHaveBeenLastCalledWith({left: 83, centerY: 14});
    vi.mocked(invoke).mockResolvedValueOnce({left: 91, centerY: 22});
    native.onResized.mock.calls[0][0]();
    await vi.waitFor(() => expect(anchor).toHaveBeenLastCalledWith({left: 91, centerY: 22}));
    vi.mocked(invoke).mockRejectedValueOnce(new Error("window closed"));
    native.onResized.mock.calls[0][0]();
    await vi.waitFor(() => expect(error).toHaveBeenCalledOnce());
    cleanup(); expect(unlisten).toHaveBeenCalledOnce();
  });
  it("routes Windows controls and reports resize failures", async () => {
    const chrome = createWindowChrome();
    const update = vi.fn(); const error = vi.fn();
    const cleanup = await chrome.initialize(update, error);
    expect(native.setDecorations).toHaveBeenCalledWith(false);
    expect(invoke).not.toHaveBeenCalled();
    expect(update).toHaveBeenCalledWith(false);
    await chrome.minimize(); await chrome.toggleMaximize(); await chrome.close();
    expect(native.minimize).toHaveBeenCalledOnce();
    expect(native.toggleMaximize).toHaveBeenCalledOnce();
    expect(native.close).toHaveBeenCalledOnce();
    native.isMaximized.mockRejectedValue(new Error("window closed"));
    native.onResized.mock.calls[0][0]();
    await vi.waitFor(() => expect(error).toHaveBeenCalled());
    cleanup(); expect(unlisten).toHaveBeenCalledOnce();
  });
});
