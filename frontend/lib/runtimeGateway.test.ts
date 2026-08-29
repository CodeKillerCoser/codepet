import { describe, expect, it } from "vitest";
import { RuntimeGatewayRequestError, runtimeGatewayErrorMessage } from "./runtimeGateway";

describe("runtimeGatewayErrorMessage", () => {
  it("keeps the protocol error code visible for diagnostics", () => {
    const error = new RuntimeGatewayRequestError({
      code: "owner_changed",
      message: "Desktop owner changed",
      retryable: true,
    });

    expect(runtimeGatewayErrorMessage(error)).toBe("[owner_changed] Desktop owner changed");
  });
});
