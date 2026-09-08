// Handwritten Provider SDK sources embedded by Bun into the standalone generator.
// Keep paths explicit for static bundling; the export test checks this list against src/.
import providerBackgroundProbe from "../../sdk/rust/codepet-provider-sdk/src/background_probe.rs" with { type: "text" };
import providerConversationQuery from "../../sdk/rust/codepet-provider-sdk/src/conversation_query.rs" with { type: "text" };
import providerLocalRuntime from "../../sdk/rust/codepet-provider-sdk/src/local_runtime.rs" with { type: "text" };
import providerProcess from "../../sdk/rust/codepet-provider-sdk/src/process.rs" with { type: "text" };
import providerProcessUnix from "../../sdk/rust/codepet-provider-sdk/src/process/unix.rs" with { type: "text" };
import providerProcessWindows from "../../sdk/rust/codepet-provider-sdk/src/process/windows.rs" with { type: "text" };
import providerContentItemText from "../../sdk/rust/codepet-provider-sdk/src/content/item_text.rs" with { type: "text" };
import providerContentMod from "../../sdk/rust/codepet-provider-sdk/src/content/mod.rs" with { type: "text" };
import providerLib from "../../sdk/rust/codepet-provider-sdk/src/lib.rs" with { type: "text" };
import providerMessageCodec from "../../sdk/rust/codepet-provider-sdk/src/message/codec.rs" with { type: "text" };
import providerMessageFrame from "../../sdk/rust/codepet-provider-sdk/src/message/frame.rs" with { type: "text" };
import providerMessageMod from "../../sdk/rust/codepet-provider-sdk/src/message/mod.rs" with { type: "text" };
import providerMessageMux from "../../sdk/rust/codepet-provider-sdk/src/message/mux.rs" with { type: "text" };
import providerRuntimeEvents from "../../sdk/rust/codepet-provider-sdk/src/runtime/events.rs" with { type: "text" };
import providerRuntimeHeartbeat from "../../sdk/rust/codepet-provider-sdk/src/runtime/heartbeat.rs" with { type: "text" };
import providerRuntimeIo from "../../sdk/rust/codepet-provider-sdk/src/runtime/io.rs" with { type: "text" };
import providerRuntimeMod from "../../sdk/rust/codepet-provider-sdk/src/runtime/mod.rs" with { type: "text" };
import providerRuntimeMux from "../../sdk/rust/codepet-provider-sdk/src/runtime/mux.rs" with { type: "text" };
import providerRuntimeOptions from "../../sdk/rust/codepet-provider-sdk/src/runtime/options.rs" with { type: "text" };
import providerRuntimeStdio from "../../sdk/rust/codepet-provider-sdk/src/runtime/stdio.rs" with { type: "text" };
import providerTransportError from "../../sdk/rust/codepet-provider-sdk/src/transport/error.rs" with { type: "text" };
import providerTransportFrame from "../../sdk/rust/codepet-provider-sdk/src/transport/frame.rs" with { type: "text" };
import providerTransportHandshake from "../../sdk/rust/codepet-provider-sdk/src/transport/handshake.rs" with { type: "text" };
import providerTransportMod from "../../sdk/rust/codepet-provider-sdk/src/transport/mod.rs" with { type: "text" };
import providerTransportMuxBudget from "../../sdk/rust/codepet-provider-sdk/src/transport/mux/budget.rs" with { type: "text" };
import providerTransportMuxIo from "../../sdk/rust/codepet-provider-sdk/src/transport/mux/io.rs" with { type: "text" };
import providerTransportMuxMod from "../../sdk/rust/codepet-provider-sdk/src/transport/mux/mod.rs" with { type: "text" };
import providerTransportMuxTests from "../../sdk/rust/codepet-provider-sdk/src/transport/mux/tests.rs" with { type: "text" };

import providerRuntimeActivity from "../../sdk/rust/codepet-provider-sdk/src/runtime/activity.rs" with { type: "text" };

export const providerRuntimeFiles = new Map([
  ["background_probe.rs", providerBackgroundProbe],
  ["conversation_query.rs", providerConversationQuery],
  ["local_runtime.rs", providerLocalRuntime],
  ["process.rs", providerProcess],
  ["process/unix.rs", providerProcessUnix],
  ["process/windows.rs", providerProcessWindows],
  ["content/item_text.rs", providerContentItemText],
  ["content/mod.rs", providerContentMod],
  ["lib.rs", providerLib],
  ["message/codec.rs", providerMessageCodec],
  ["message/frame.rs", providerMessageFrame],
  ["message/mod.rs", providerMessageMod],
  ["message/mux.rs", providerMessageMux],
  ["runtime/events.rs", providerRuntimeEvents],
  ["runtime/activity.rs", providerRuntimeActivity],
  ["runtime/heartbeat.rs", providerRuntimeHeartbeat],
  ["runtime/io.rs", providerRuntimeIo],
  ["runtime/mod.rs", providerRuntimeMod],
  ["runtime/mux.rs", providerRuntimeMux],
  ["runtime/options.rs", providerRuntimeOptions],
  ["runtime/stdio.rs", providerRuntimeStdio],
  ["transport/error.rs", providerTransportError],
  ["transport/frame.rs", providerTransportFrame],
  ["transport/handshake.rs", providerTransportHandshake],
  ["transport/mod.rs", providerTransportMod],
  ["transport/mux/budget.rs", providerTransportMuxBudget],
  ["transport/mux/io.rs", providerTransportMuxIo],
  ["transport/mux/mod.rs", providerTransportMuxMod],
  ["transport/mux/tests.rs", providerTransportMuxTests],
]);
