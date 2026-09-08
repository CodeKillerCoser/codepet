import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { codepetGateway } from "./codepetGateway";
vi.mock("@tauri-apps/api/core", () => ({invoke: vi.fn()}));
afterEach(() => vi.mocked(invoke).mockReset());
it("uses canonical JSON-RPC through the shared Gateway bridge", async () => {
  vi.mocked(invoke).mockImplementation(async (command, args: any) => {
    expect(command).toBe("codepet_gateway_request");
    expect(args.request).toMatchObject({jsonrpc: "2.0", method: "provider.list", params: {}});
    return {jsonrpc: "2.0", id: args.request.id, result: {providers: []}};
  });
  expect(await codepetGateway("provider.list", {})).toEqual({providers: []});
});
it("rejects mismatched replies and surfaces protocol errors", async () => {
  vi.mocked(invoke).mockResolvedValue({jsonrpc: "2.0", id: "other", result: {}});
  await expect(codepetGateway("provider.list", {})).rejects.toThrow("不匹配");
  vi.mocked(invoke).mockImplementation(async (_command, args: any) => ({jsonrpc: "2.0", id: args.request.id, error: {code: -32000, message: "usage unavailable"}}));
  await expect(codepetGateway("provider.list", {})).rejects.toThrow("usage unavailable");
});
