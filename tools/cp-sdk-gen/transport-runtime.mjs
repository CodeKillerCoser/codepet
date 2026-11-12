// Static embedding: independent transport runtime exported with the Rust SDK.
import source0 from "../../sdk/rust/codepet-transport/src/lib.rs" with { type: "text" };
import source1 from "../../sdk/rust/codepet-transport/src/payload.rs" with { type: "text" };
import source2 from "../../sdk/rust/codepet-transport/src/profile.rs" with { type: "text" };
import source3 from "../../sdk/rust/codepet-transport/src/serial.rs" with { type: "text" };
import source4 from "../../sdk/rust/codepet-transport/src/tests.rs" with { type: "text" };
import source5 from "../../sdk/rust/codepet-transport/src/transport/error.rs" with { type: "text" };
import source6 from "../../sdk/rust/codepet-transport/src/transport/frame.rs" with { type: "text" };
import source7 from "../../sdk/rust/codepet-transport/src/transport/handshake.rs" with { type: "text" };
import source8 from "../../sdk/rust/codepet-transport/src/transport/mod.rs" with { type: "text" };
import source9 from "../../sdk/rust/codepet-transport/src/transport/mux/budget.rs" with { type: "text" };
import source10 from "../../sdk/rust/codepet-transport/src/transport/mux/io.rs" with { type: "text" };
import source11 from "../../sdk/rust/codepet-transport/src/transport/mux/mod.rs" with { type: "text" };
import source12 from "../../sdk/rust/codepet-transport/src/transport/mux/tests.rs" with { type: "text" };
import protocolSource from "../../sdk/rust/codepet-transport/src/protocol.rs" with { type: "text" };
export const transportRuntimeFiles = new Map([
  ["protocol.rs", protocolSource],
  ["lib.rs", source0],
  ["payload.rs", source1],
  ["profile.rs", source2],
  ["serial.rs", source3],
  ["tests.rs", source4],
  ["transport/error.rs", source5],
  ["transport/frame.rs", source6],
  ["transport/handshake.rs", source7],
  ["transport/mod.rs", source8],
  ["transport/mux/budget.rs", source9],
  ["transport/mux/io.rs", source10],
  ["transport/mux/mod.rs", source11],
  ["transport/mux/tests.rs", source12],
]);
