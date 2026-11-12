import { Duplex } from "node:stream";
import { Server } from "@bbk47/yamux";
import {
  methods as methodTable,
  handshakeSchema,
  validateHello,
  validateLimits as validateLimitsShape,
} from "./contracts.mjs";
const controlMethods = new Set(
  methodTable.filter((m) => m.dispatchLane === "control").map((m) => m.name),
);
export const features = [
  "raw-json",
  "zstd-json",
  "yamux-1",
  "message-grant",
  "reserved-lanes",
  "bounded-decode",
  "reset",
  "ordered-events",
];
export const limits = Object.fromEntries(
  Object.entries(handshakeSchema.$defs.ProviderTransportLimits.properties).map(
    ([key, schema]) => [key, schema.default],
  ),
);
export class Reader {
  constructor(stream, max = limits.maxEncodedMessageBytes) {
    this.stream = stream;
    this.max = max;
    this.buffer = Buffer.alloc(0);
    this.ended = false;
    this.error = null;
    this.waiter = null;
    this.onData = (c) => {
      if (this.buffer.length + c.length > this.max) {
        this.error = Error("read buffer exceeded");
        this.wake();
        return;
      }
      this.buffer = Buffer.concat([this.buffer, c]);
      this.wake();
    };
    this.onEnd = () => {
      this.ended = true;
      this.wake();
    };
    this.onError = (e) => {
      this.error = e;
      this.wake();
    };
    stream.on("data", this.onData);
    stream.on("end", this.onEnd);
    stream.on("error", this.onError);
    stream.on("close", this.onEnd);
  }
  wake() {
    this.waiter?.();
    this.waiter = null;
  }
  async read(n, timeout = limits.streamTimeoutMs) {
    if (n > this.max) throw Error("read size exceeded");
    const end = Date.now() + timeout;
    while (this.buffer.length < n) {
      if (this.error) throw this.error;
      if (this.ended) throw Error("truncated stream");
      const left = end - Date.now();
      if (left <= 0) throw Error("stream timeout");
      await new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          this.waiter = null;
          reject(Error("stream timeout"));
        }, left);
        this.waiter = () => {
          clearTimeout(timer);
          resolve();
        };
      });
    }
    const result = this.buffer.subarray(0, n);
    this.buffer = this.buffer.subarray(n);
    return result;
  }
  detach() {
    this.stream.off("data", this.onData);
    this.stream.off("end", this.onEnd);
    this.stream.off("error", this.onError);
    this.stream.off("close", this.onEnd);
    return this.buffer;
  }
}
export const write = (stream, b) =>
  new Promise((resolve, reject) =>
    stream.write(b, (e) => (e ? reject(e) : resolve())),
  );
export function bootstrapFrame(phase, payload) {
  const body = Buffer.from(JSON.stringify(payload));
  const h = Buffer.alloc(8);
  h.write("CPMX");
  h[4] = 1;
  h[5] = phase;
  h.writeUInt16BE(body.length, 6);
  return Buffer.concat([h, body]);
}
function validateLimits(value) {
  if (!validateLimitsShape(value)) throw Error("invalid transport limits");
  if (
    value.receiveBudgetBytes <
      value.maxEncodedMessageBytes + value.maxDecodedMessageBytes ||
    value.smallReceiveBudgetBytes < 2 * value.smallMessageBytes ||
    value.controlReceiveBudgetBytes < 2 * value.controlMessageBytes ||
    value.controlMessageBytes > value.smallMessageBytes ||
    value.smallMessageBytes > value.maxDecodedMessageBytes ||
    value.idleTimeoutMs > value.streamTimeoutMs
  )
    throw Error("inconsistent limits");
}
export async function start({ handle, frame, unframe, writeBuffer, setSend }) {
  const bootstrapReader = new Reader(process.stdin, 8192);
  const deadline = Date.now() + 3000;
  async function readBootstrap(phase) {
    const h = await bootstrapReader.read(8, deadline - Date.now());
    if (
      h.toString("ascii", 0, 4) !== "CPMX" ||
      h[4] !== 1 ||
      h[5] !== phase ||
      h.readUInt16BE(6) > 4096
    )
      throw Error("invalid bootstrap");
    return JSON.parse(
      new TextDecoder("utf-8", { fatal: true }).decode(
        await bootstrapReader.read(h.readUInt16BE(6), deadline - Date.now()),
      ),
    );
  }
  const hello = await readBootstrap(1);
  if (
    !validateHello(hello) ||
    !hello.supportedVersions.includes(1) ||
    new Set(hello.features).size !== hello.features.length ||
    features.some((f) => !hello.features.includes(f))
  )
    throw Error("mandatory feature/version missing");
  validateLimits(hello.receive);
  const peer = hello.receive;
  await writeBuffer(
    bootstrapFrame(2, { selectedVersion: 1, features, receive: limits }),
  );
  if ((await readBootstrap(3)) !== 1) throw Error("invalid confirm");
  await writeBuffer(bootstrapFrame(4, 1));
  process.stdin.pause();
  const remaining = bootstrapReader.detach();
  const transport = new Duplex({
    read() {
      process.stdin.resume();
    },
    write(chunk, encoding, callback) {
      writeBuffer(chunk).then(() => callback(), callback);
    },
  });
  process.stdin.on("data", (c) => {
    if (!transport.push(c)) process.stdin.pause();
  });
  process.stdin.on("end", () => transport.push(null));
  process.stdin.on("error", (e) => transport.destroy(e));
  const session = Server(transport, {
    initialStreamWindow: 262144,
    maxFrameSize: Math.min(
      limits.maxFramePayloadBytes,
      peer.maxFramePayloadBytes,
    ),
  });
  session.on("error", (e) => {
    process.stderr.write(`yamux: ${e.message}\n`);
    process.exit(1);
  });
  transport.on("end", () => {
    setImmediate(() => process.exit(0));
  });
  if (remaining.length) transport.push(remaining);
  process.stdin.resume();
  const used = [
    { streams: 0, bytes: 0 },
    { streams: 0, bytes: 0 },
    { streams: 0, bytes: 0 },
  ];
  const maxSize = (cls, l) =>
    cls === 2
      ? l.controlMessageBytes
      : cls === 1
        ? l.smallMessageBytes
        : l.maxDecodedMessageBytes;
  const streamCap = [
      limits.normalStreams,
      limits.smallStreams,
      limits.controlStreams,
    ],
    budget = [
      limits.receiveBudgetBytes,
      limits.smallReceiveBudgetBytes,
      limits.controlReceiveBudgetBytes,
    ];
  async function receive(reader, stream) {
    const h = await reader.read(10);
    const cls = h[1],
      encoded = h.readUInt32BE(2),
      decoded = h.readUInt32BE(6);
    if (
      h[0] !== 1 ||
      cls > 2 ||
      encoded < 10 ||
      encoded > limits.maxEncodedMessageBytes ||
      decoded > maxSize(cls, limits) ||
      used[cls].streams >= streamCap[cls] ||
      used[cls].bytes + encoded + decoded > budget[cls]
    )
      throw Error("invalid envelope or receive capacity exceeded");
    used[cls].streams++;
    used[cls].bytes += encoded + decoded;
    const release = () => {
      used[cls].streams--;
      used[cls].bytes -= encoded + decoded;
    };
    try {
      await write(stream, Buffer.from([1]));
      const message = await unframe(await reader.read(encoded), decoded);
      return { message, cls, release };
    } catch (e) {
      release();
      throw e;
    }
  }
  async function sendMessage(stream, reader, message, forceClass) {
    let raw = Buffer.from(JSON.stringify(message)),
      cls = forceClass ?? (raw.length <= peer.smallMessageBytes ? 1 : 0);
    if (
      raw.length > maxSize(cls, peer) ||
      raw.length > peer.maxDecodedMessageBytes
    ) {
      if (!message.id) throw Error("event exceeds peer message limit");
      message = {
        jsonrpc: "2.0",
        id: message.id,
        error: {
          code: -32000,
          message: "reply exceeds negotiated limit",
          data: {
            code: "message_too_large",
            message: "reply exceeds negotiated limit",
            retryable: false,
          },
        },
      };
      raw = Buffer.from(JSON.stringify(message));
    }
    let { bytes, rawLength } = await frame(
      message,
      raw.length >= 32768 ? 1 : 0,
    );
    // Only control replies retain the request reserve. Other messages select
    // their own lane from both encoded and decoded sizes.
    if (forceClass !== 2)
      cls =
        rawLength <= peer.smallMessageBytes &&
        bytes.length <= peer.smallMessageBytes
          ? 1
          : 0;
    if (bytes.length > peer.maxEncodedMessageBytes)
      throw Error("encoded message exceeds peer limit");
    const h = Buffer.alloc(10);
    h[0] = 1;
    h[1] = cls;
    h.writeUInt32BE(bytes.length, 2);
    h.writeUInt32BE(rawLength, 6);
    await write(stream, h);
    if ((await reader.read(1, peer.streamTimeoutMs))[0] !== 1)
      throw Error("missing grant");
    await write(stream, bytes);
  }
  session.on("stream", (stream) => {
    const reader = new Reader(stream);
    stream.on("error", () => {});
    void (async () => {
      let release;
      try {
        const received = await receive(reader, stream);
        release = received.release;
        if (
          controlMethods.has(received.message.method) !==
          (received.cls === 2)
        )
          throw Error("wrong dispatch lane");
        await handle(received.message, (response) =>
          sendMessage(
            stream,
            reader,
            response,
            received.cls === 2 ? 2 : undefined,
          ),
        );
        stream.end();
      } catch (e) {
        stream.destroy(e);
      } finally {
        release?.();
        reader.detach();
      }
    })();
  });
  let eventsChain = Promise.resolve();
  setSend((message) => {
    eventsChain = eventsChain.then(async () => {
      const stream = session.openStream();
      const reader = new Reader(stream);
      stream.on("error", () => {});
      try {
        await sendMessage(stream, reader, message);
        if ((await reader.read(1, peer.streamTimeoutMs))[0] !== 2)
          throw Error("event ACK missing");
        stream.end();
      } finally {
        reader.detach();
      }
    });
    return eventsChain;
  });
}
