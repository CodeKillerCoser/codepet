// A dependency protocol fixture; never calls a model.
let buffer = "";
const emit = (m) => process.stdout.write(JSON.stringify(m) + "\n");
process.stdin.setEncoding("utf8");
process.stdin.on("data", (c) => {
  buffer += c;
  for (;;) {
    const n = buffer.indexOf("\n");
    if (n < 0) return;
    const m = JSON.parse(buffer.slice(0, n));
    buffer = buffer.slice(n + 1);
    if (m.type === "get_state")
      emit({
        type: "response",
        id: m.id,
        success: true,
        data: {
          model: { provider: "fixture", id: "config-selected" },
          unusedSecret: "must-not-forward",
        },
      });
    else if (m.type === "prompt") {
      emit({ type: "response", id: m.id, success: true });
      emit({
        type: "message_update",
        assistantMessageEvent: { type: "text_delta", delta: m.message },
      });
      emit({
        type: "agent_end",
        messages: [{ role: "assistant", stopReason: "stop" }],
      });
    } else emit({ type: "response", id: m.id, success: true });
  }
});
