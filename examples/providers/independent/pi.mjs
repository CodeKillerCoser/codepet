// Pi RPC adapter. Reads only allowlisted output fields; never opens pi settings/auth files.
import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
const children = new Set();
process.once("exit", () => {
  for (const child of children) child.kill();
});
export class PiSession extends EventEmitter {
  constructor({
    cwd,
    command = process.env.ECHO_PI_COMMAND || "pi",
    args = [],
  }) {
    super();
    this.next = 0;
    this.pending = new Map();
    this.output = Buffer.alloc(0);
    this.closed = false;
    this.child = spawn(command, [...args, "--mode", "rpc", "--no-session"], {
      cwd,
      stdio: ["pipe", "pipe", "pipe"],
    });
    children.add(this.child);
    this.child.stderr.resume();
    this.child.on("error", () =>
      this.failure(Error("pi process could not start")),
    );
    this.child.on("exit", () => {
      children.delete(this.child);
      this.closed = true;
      this.failure(Error("pi process exited"));
    });
    this.child.stdout.on("data", (chunk) => {
      try {
        if (this.output.length + chunk.length > 16 * 1024 * 1024)
          throw Error("pi record limit exceeded");
        this.output = Buffer.concat([this.output, chunk]);
        for (;;) {
          const n = this.output.indexOf(10);
          if (n < 0) break;
          const record = this.output.subarray(0, n);
          this.output = this.output.subarray(n + 1);
          this.onRecord(
            JSON.parse(
              new TextDecoder("utf-8", { fatal: true }).decode(record),
            ),
          );
        }
      } catch {
        this.failure(Error("invalid pi RPC output"));
        this.close();
      }
    });
  }
  failure(error) {
    for (const { reject, timer } of this.pending.values()) {
      clearTimeout(timer);
      reject(error);
    }
    this.pending.clear();
    this.emit("failed", error);
  }
  onRecord(record) {
    if (record.type === "response" && typeof record.id === "string") {
      const pending = this.pending.get(record.id);
      if (!pending) return;
      this.pending.delete(record.id);
      clearTimeout(pending.timer);
      if (record.success) pending.resolve(record.data);
      else pending.reject(Error(`pi ${pending.type} rejected`));
      return;
    }
    if (
      record.type === "message_update" &&
      record.assistantMessageEvent?.type === "text_delta" &&
      typeof record.assistantMessageEvent.delta === "string"
    )
      this.emit("delta", record.assistantMessageEvent.delta);
    if (record.type === "agent_end") {
      const failed = record.messages?.some(
        (m) => m.role === "assistant" && m.stopReason === "error",
      );
      this.emit("done", failed ? Error("pi model execution failed") : null);
    }
    if (
      record.type === "extension_ui_request" &&
      ["select", "confirm", "input", "editor"].includes(record.method)
    )
      this.child.stdin.write(
        JSON.stringify({
          type: "extension_ui_response",
          id: record.id,
          cancelled: true,
        }) + "\n",
      );
  }
  command(type, fields = {}) {
    if (this.closed) return Promise.reject(Error("pi process closed"));
    const id = `pi-${++this.next}`;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(Error(`pi ${type} timeout`));
      }, 30000);
      this.pending.set(id, { resolve, reject, timer, type });
      this.child.stdin.write(
        JSON.stringify({ id, type, ...fields }) + "\n",
        (error) => {
          if (error) {
            clearTimeout(timer);
            this.pending.delete(id);
            reject(Error("pi input failed"));
          }
        },
      );
    });
  }
  async state() {
    const state = await this.command("get_state");
    return {
      model:
        typeof state?.model?.provider === "string" &&
        typeof state?.model?.id === "string"
          ? `${state.model.provider}/${state.model.id}`
          : undefined,
    };
  }
  async run(text, onDelta) {
    let chain = Promise.resolve();
    const delta = (value) => {
      chain = chain.then(() => onDelta(value));
    };
    return new Promise((resolve, reject) => {
      const cleanup = () => {
        this.off("delta", delta);
        this.off("done", done);
        this.off("failed", failed);
      };
      const done = (error) => {
        cleanup();
        chain.then(() => (error ? reject(error) : resolve()), reject);
      };
      const failed = (error) => {
        cleanup();
        reject(error);
      };
      this.on("delta", delta);
      this.once("done", done);
      this.once("failed", failed);
      this.command("prompt", { message: text }).catch(failed);
    });
  }
  async abort() {
    await this.command("abort");
  }
  close() {
    this.closed = true;
    this.child.kill();
  }
}
