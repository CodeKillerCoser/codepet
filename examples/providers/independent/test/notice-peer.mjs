import { spawn } from "node:child_process";
import { Duplex } from "node:stream";
import zlib from "node:zlib";
import { Client } from "@bbk47/yamux";
import { Reader, write, bootstrapFrame, limits, features } from "../mux.mjs";
import { requestValidators, responseValidators } from "../contracts.mjs";
export const DROP = Symbol("no Host response");
export class Peer {
  constructor(
    profile,
    env = {},
    onRequest = async () => {
      throw Error("unexpected provider request");
    },
  ) {
    this.profile = profile;
    this.onRequest = onRequest;
    this.events = [];
    this.requests = [];
    this.errors = [];
    this.next = 0;
    this.pending = new Map();
    this.child = spawn(process.execPath, ["provider.mjs"], {
      cwd: process.env.PROVIDER_TEST_DIRECTORY || process.cwd(),
      env: {
        ...process.env,
        CODEPET_PROVIDER_TRANSPORT: profile,
        ECHO_BACKEND: "notice",
        ECHO_PLUGIN_ID: "independent.notice.fixture",
        ...env,
      },
    });
    this.stderr = "";
    this.child.stderr.on("data", (d) => (this.stderr += d));
  }
  async connect() {
    if (this.profile === "stdio-codepet-mux-v1") {
      const r = new Reader(this.child.stdout);
      await write(
        this.child.stdin,
        bootstrapFrame(1, {
          supportedVersions: [1],
          features,
          receive: limits,
        }),
      );
      let h = await r.read(8);
      await r.read(h.readUInt16BE(6));
      await write(this.child.stdin, bootstrapFrame(3, 1));
      h = await r.read(8);
      await r.read(h.readUInt16BE(6));
      this.child.stdout.pause();
      const remaining = r.detach();
      const transport = new Duplex({
        read: () => this.child.stdout.resume(),
        write: (b, e, cb) => this.child.stdin.write(b, cb),
      });
      this.child.stdout.on("data", (b) => {
        if (!transport.push(b)) this.child.stdout.pause();
      });
      this.child.stdout.on("end", () => transport.push(null));
      this.mux = Client(transport, {
        initialStreamWindow: 262144,
        maxFrameSize: 16384,
      });
      this.mux.on("error", (e) => this.errors.push(e));
      this.mux.on("stream", (stream) => {
        stream.on("error", () => {});
        const reader = new Reader(stream);
        void (async () => {
          try {
            const m = await this.readMessage(stream, reader);
            if (!m.id) {
              this.events.push(m);
              await write(stream, Buffer.from([2]));
            } else {
              const response = await this.providerRequest(m);
              if (response === DROP) return;
              await this.sendMessage(stream, reader, response);
            }
            stream.end();
          } catch (e) {
            this.errors.push(e);
          } finally {
            reader.detach();
          }
        })();
      });
      if (remaining.length) transport.push(remaining);
      this.child.stdout.resume();
    } else {
      let buffer = Buffer.alloc(0);
      this.child.stdout.on("data", (chunk) => {
        try {
          buffer = Buffer.concat([buffer, chunk]);
          for (;;) {
            let raw;
            if (this.profile === "stdio-jsonrpc-v1") {
              const n = buffer.indexOf(10);
              if (n < 0) break;
              raw = buffer.subarray(0, n);
              buffer = buffer.subarray(n + 1);
            } else {
              if (buffer.length < 10) break;
              const n = buffer.readUInt32BE(6) + 10;
              if (buffer.length < n) break;
              raw = zlib.zstdDecompressSync(buffer.subarray(10, n));
              buffer = buffer.subarray(n);
            }
            const message = JSON.parse(raw);
            if (message.method && message.id)
              void this.providerRequest(message)
                .then((response) => {
                  if (response !== DROP) this.sendRecord(response);
                })
                .catch((e) => this.errors.push(e));
            else if (message.id) {
              this.pending.get(message.id)?.(message);
              this.pending.delete(message.id);
            } else this.events.push(message);
          }
        } catch (e) {
          this.errors.push(e);
        }
      });
    }
    return this;
  }
  async providerRequest(message) {
    this.requests.push(message);
    const v = requestValidators.get(message.method);
    if (!v?.(message.params))
      throw Error(
        `invalid outgoing ${message.method}: ${JSON.stringify(v?.errors)}`,
      );
    try {
      const result = await this.onRequest(message);
      if (result === DROP) return DROP;
      const validate = responseValidators.get(message.method);
      if (!validate(result))
        throw Error(
          `invalid test Host result: ${JSON.stringify(validate.errors)}`,
        );
      return { jsonrpc: "2.0", id: message.id, result };
    } catch (e) {
      return {
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32000,
          message: e.message,
          data: {
            code: e.code || "fixture_host_error",
            message: e.message,
            retryable: false,
          },
        },
      };
    }
  }
  sendRecord(message) {
    const raw = Buffer.from(JSON.stringify(message));
    if (this.profile === "stdio-jsonrpc-v1")
      return this.child.stdin.write(Buffer.concat([raw, Buffer.from("\n")]));
    const body = zlib.zstdCompressSync(raw),
      header = Buffer.alloc(10);
    header.write("CPRF");
    header[4] = header[5] = 1;
    header.writeUInt32BE(body.length, 6);
    this.child.stdin.write(Buffer.concat([header, body]));
  }
  async sendMessage(stream, reader, message) {
    const raw = Buffer.from(JSON.stringify(message)),
      body = raw.length >= 32768 ? zlib.zstdCompressSync(raw) : raw;
    const frame = Buffer.alloc(10 + body.length);
    frame.write("CPRF");
    frame[4] = 1;
    frame[5] = raw.length >= 32768 ? 1 : 0;
    frame.writeUInt32BE(body.length, 6);
    body.copy(frame, 10);
    const h = Buffer.alloc(10);
    h[0] = 1;
    h[1] = ["provider.ping", "provider.shutdown"].includes(message.method)
      ? 2
      : raw.length <= 65536 && frame.length <= 65536
        ? 1
        : 0;
    h.writeUInt32BE(frame.length, 2);
    h.writeUInt32BE(raw.length, 6);
    await write(stream, h);
    if ((await reader.read(1))[0] !== 1) throw Error("grant missing");
    await write(stream, frame);
  }
  async readMessage(stream, reader) {
    const h = await reader.read(10);
    await write(stream, Buffer.from([1]));
    const f = await reader.read(h.readUInt32BE(2));
    const raw = f[5] ? zlib.zstdDecompressSync(f.subarray(10)) : f.subarray(10);
    if (raw.length !== h.readUInt32BE(6)) throw Error("decoded size mismatch");
    return JSON.parse(raw);
  }
  async exchange(method, params = {}) {
    const message = {
      jsonrpc: "2.0",
      id: `host-${++this.next}`,
      method,
      params,
    };
    if (this.mux) {
      const stream = this.mux.openStream();
      stream.on("error", () => {});
      const r = new Reader(stream);
      try {
        await this.sendMessage(stream, r, message);
        const reply = await this.readMessage(stream, r);
        stream.end();
        return reply;
      } finally {
        r.detach();
      }
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => reject(Error(`${method} timeout: ${this.stderr}`)),
        3000,
      );
      this.pending.set(message.id, (m) => {
        clearTimeout(timer);
        resolve(m);
      });
      this.sendRecord(message);
    });
  }
  async rpc(method, params = {}) {
    const response = await this.exchange(method, params);
    if (response.error)
      throw Object.assign(Error(response.error.message), {
        code: response.error.data?.code,
        rpcCode: response.error.code,
      });
    return response.result;
  }
  async initialize() {
    return this.rpc("provider.initialize", {
      hostClientId: "fixture",
      hostDeviceId: "fixture",
      hostVersion: "test",
      protocol: { version: 4, minVersion: 4 },
    });
  }
  close() {
    this.child.kill();
  }
}
export const waitUntil = async (predicate) => {
  const deadline = Date.now() + 3000;
  while (!predicate()) {
    if (Date.now() > deadline) throw Error("condition timeout");
    await new Promise((r) => setTimeout(r, 5));
  }
};
export function hostStore() {
  const notices = new Map(),
    operations = new Map();
  let next = 0;
  return {
    notices,
    handle: async (message) => {
      const p = message.params;
      if (message.method === "notice.create") {
        const key = p.sourceId + ":" + p.creationKey;
        if (!notices.has(key))
          notices.set(key, {
            noticeId: `notice-${++next}`,
            pluginId: "independent.notice.fixture",
            sourceId: p.sourceId,
            status: p.status,
            content: p.content,
            actions: p.actions || [],
            revision: 1,
            finished: false,
            createdAt: Date.now(),
            updatedAt: Date.now(),
          });
        return { notice: structuredClone(notices.get(key)) };
      }
      const notice = [...notices.values()].find(
        (n) => n.noticeId === p.noticeId,
      );
      if (!notice) throw Error("missing notice");
      if (operations.has(p.operationId))
        return { notice: structuredClone(operations.get(p.operationId)) };
      if (p.expectedRevision !== notice.revision)
        throw Error("revision conflict");
      if (message.method === "notice.update" && notice.finished)
        throw Error("cannot reopen");
      for (const key of ["status", "content", "actions"])
        if (p[key] !== undefined) notice[key] = p[key];
      if (message.method === "notice.finish") notice.finished = true;
      notice.revision++;
      notice.updatedAt = Date.now();
      operations.set(p.operationId, structuredClone(notice));
      return { notice: structuredClone(notice) };
    },
  };
}
