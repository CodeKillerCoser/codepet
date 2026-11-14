import fs from "node:fs";
import zlib from "node:zlib";
import { promisify } from "node:util";
import { PiSession } from "./pi.mjs";
import { normalizeSelection } from "./selection.mjs";
import {
  requestValidators,
  responseValidators,
  eventValidators,
} from "./contracts.mjs";
const arg = (key, fallback) => {
  let i = process.argv.indexOf(`--${key}`);
  return i < 0 ? fallback : process.argv[i + 1];
};
const pluginId = arg(
  "plugin-id",
  process.env.ECHO_PLUGIN_ID || "independent.echo.jsonl",
);
const numeric = (name, fallback) =>
  Number(
    arg(
      name,
      process.env[`ECHO_${name.replaceAll("-", "_").toUpperCase()}`] ||
        fallback,
    ),
  );
// Business revision is independent of software, SDK and transport versions.
// Keep a declared 2/2 mode for standalone legacy-peer acceptance.
const protocolVersion = numeric("protocol-version", 3);
if (![2, 3].includes(protocolVersion))
  throw Error("unsupported business protocol fixture version");
const controls = {
  describeDelayMs: numeric("describe-delay-ms", 0),
  turnDelayMs: numeric("turn-delay-ms", 30),
  chunkSize: numeric("chunk-size", 16),
  largeEventBytes: numeric("large-event-bytes", 0),
  partialBytes: numeric("partial-bytes", 0),
  partialDelayMs: numeric("partial-delay-ms", 0),
  stallAfterPartial: numeric("stall-after-partial", 0),
};
const backend = process.env.ECHO_BACKEND || "echo";
const kind = backend === "pi" ? "independent-pi" : "independent-echo";
let dataDirectory;
const profile =
  process.env.CODEPET_PROVIDER_TRANSPORT ||
  arg("transport", "stdio-jsonrpc-v1");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const compress = promisify(zlib.zstdCompress),
  decompress = promisify(zlib.zstdDecompress);
const MAX_ENCODED = 16 * 1024 * 1024,
  MAX_DECODED = 128 * 1024 * 1024;
const fatal = (e) => {
  process.stderr.write(`independent-provider: ${e.message}\n`);
  process.exit(1);
};
process.on("uncaughtException", fatal);
process.on("unhandledRejection", fatal);
let writeChain = Promise.resolve();
function writeBuffer(buf) {
  writeChain = writeChain.then(async () => {
    const n = controls.partialBytes;
    if (n > 0 && buf.length > n) {
      await rawWrite(buf.subarray(0, n));
      if (controls.stallAfterPartial) await new Promise(() => {});
      if (controls.partialDelayMs) await sleep(controls.partialDelayMs);
      await rawWrite(buf.subarray(n));
    } else await rawWrite(buf);
  });
  return writeChain;
}
function rawWrite(buf) {
  return new Promise((resolve, reject) =>
    process.stdout.write(buf, (e) => (e ? reject(e) : resolve())),
  );
}
async function frame(obj, encoding = 1) {
  const raw = Buffer.from(JSON.stringify(obj));
  if (raw.length > MAX_DECODED) throw Error("decoded output too large");
  const body = encoding
    ? await compress(raw, {
        params: { [zlib.constants.ZSTD_c_compressionLevel]: 1 },
      })
    : raw;
  const h = Buffer.alloc(10);
  h.write("CPRF");
  h[4] = 1;
  h[5] = encoding;
  h.writeUInt32BE(body.length, 6);
  if (body.length + 10 > MAX_ENCODED) throw Error("encoded output too large");
  return { bytes: Buffer.concat([h, body]), rawLength: raw.length };
}
async function unframe(buf, decodedLength) {
  if (
    buf.length < 10 ||
    buf.toString("ascii", 0, 4) !== "CPRF" ||
    buf[4] !== 1 ||
    ![0, 1].includes(buf[5]) ||
    buf.readUInt32BE(6) !== buf.length - 10
  )
    throw Error("invalid CPRF record");
  const raw = buf[5]
    ? await decompress(buf.subarray(10), {
        maxOutputLength: Math.min(decodedLength ?? MAX_DECODED, MAX_DECODED),
      })
    : buf.subarray(10);
  if (decodedLength !== undefined && raw.length !== decodedLength)
    throw Error("decoded length mismatch");
  return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(raw));
}
let send;
const capabilities = {
  revision: backend === "pi" ? "independent-pi-v2" : "independent-echo-v2",
  methods: [
    "conversation.create",
    "conversation.get",
    "conversation.list",
    "conversation.releaseInteraction",
    "turn.start",
    "turn.interrupt",
  ],
  extensions: [],
  turnSend: {
    accessMode: {
      options: [{ id: "default", displayName: "Default" }],
      defaultId: "default",
    },
    modelCatalog: {
      kind: "flat",
      models: [{ id: "echo", displayName: "Deterministic Echo" }],
      defaultSelection: { kind: "flat", modelId: "echo" },
    },
  },
  conversationCreate: { supportsTitle: true },
};
if (backend === "pi") {
  capabilities.turnSend = {
    accessMode: {
      options: [{ id: "default", displayName: "Pi configured access" }],
      defaultId: "default",
    },
  };
  capabilities.conversationCreate = { supportsTitle: true };
}
capabilities.conversationCreate.selection = capabilities.turnSend;
const plugin = {
  pluginId,
  displayName:
    backend === "pi" ? "Independent Pi RPC" : "Independent JavaScript Echo",
  version: "3.0.0",
  protocol: { version: protocolVersion, minVersion: 2 },
  ...(protocolVersion >= 3 ? { features: ["agent"] } : {}),
  instanceKinds: [kind],
};
let initialized = false,
  pingState,
  closing = false,
  sequence = 0;
const instances = new Map(),
  conversations = new Map(),
  turns = new Map(),
  accepted = new Map();
const id = (prefix) => `${prefix}-${++sequence}`;
const routed = (route, nativeResourceId) => ({
  providerId: route.providerInstanceId,
  nativeResourceId,
});
const full = (route, nativeResourceId) => ({ ...route, nativeResourceId });
const fail = (message, code = "invalid_request") => {
  throw Object.assign(Error(message), { protocolCode: code });
};
function instance(route) {
  if (route.providerPluginId !== pluginId) fail("plugin route mismatch");
  const found = instances.get(route.providerInstanceId);
  if (
    !found ||
    ["deviceId", "providerPluginId", "providerInstanceId"].some(
      (k) => route[k] !== found.value.route[k],
    )
  )
    fail("unknown instance", "not_found");
  return found;
}
function conversation(resource) {
  const inst = instance(resource);
  const found = conversations.get(
    `${resource.providerInstanceId}:${resource.nativeResourceId}`,
  );
  if (!found) fail("unknown conversation", "not_found");
  return { inst, entry: found };
}
function emit(method, params) {
  const validate = eventValidators.get(method);
  if (!validate?.(params))
    throw Error(
      `invalid generated ${method}: ${JSON.stringify(validate?.errors)}`,
    );
  return send({ jsonrpc: "2.0", method, params });
}
function item(route, conv, turn, role, text, itemId) {
  return {
    resource: routed(route, itemId),
    turn: turn.resource,
    conversation: conv.resource,
    kind: "message",
    status: "completed",
    role,
    contents: [{ contentId: `${itemId}-text`, kind: "text", text }],
  };
}
async function runTurn(entry, turn, input, inst) {
  const settings = { ...controls, ...inst.settings };
  let text = "";
  const itemId = id("assistant");
  await emit("event.turnUpserted", { turn: { ...turn } });
  const delta = async (chunk) => {
    if (turn.status === "interrupted" || closing) return;
    text += chunk;
    await emit("event.turnOutputDelta", {
      turn: full(inst.value.route, turn.resource.nativeResourceId),
      conversation: full(
        inst.value.route,
        entry.value.resource.nativeResourceId,
      ),
      itemId,
      contentId: `${itemId}-text`,
      kind: "text",
      delta: chunk,
    });
  };
  try {
    if (entry.pi) await entry.pi.run(input, delta);
    else {
      const output = input;
      const chunkSize = Math.max(1, Math.floor(Number(settings.chunkSize)));
      if (Number.isNaN(chunkSize)) throw Error("invalid chunk size");
      for (let pos = 0; pos < output.length; ) {
        await sleep(Number(settings.turnDelayMs));
        if (turn.status === "interrupted" || closing) return;
        let end = Math.min(pos + chunkSize, output.length);
        // Keep UTF-16 surrogate pairs in one delta so every JSON string is
        // valid Unicode independently, including across a configured boundary.
        if (
          end < output.length &&
          output.charCodeAt(end - 1) >= 0xd800 &&
          output.charCodeAt(end - 1) <= 0xdbff &&
          output.charCodeAt(end) >= 0xdc00 &&
          output.charCodeAt(end) <= 0xdfff
        )
          end++;
        await delta(output.slice(pos, end));
        pos = end;
      }
      if (settings.largeEventBytes)
        await delta(
          "x".repeat(
            Math.min(Number(settings.largeEventBytes), MAX_DECODED - 4096),
          ),
        );
    }
    if (turn.status === "interrupted" || closing) return;
    const assistant = item(
      inst.value.route,
      entry.value,
      turn,
      "assistant",
      text,
      itemId,
    );
    entry.items.push(assistant);
    turn.status = "completed";
    turn.completedAt = turn.updatedAt = Date.now();
    entry.value.status = "idle";
    entry.value.updatedAt = Date.now();
    delete entry.value.activeTurn;
    await emit("event.conversationItemUpserted", { item: assistant });
    await emit("event.turnUpserted", { turn });
    await emit("event.conversationUpserted", { conversation: entry.value });
  } catch (e) {
    if (turn.status === "interrupted" || closing) return;
    turn.status = "failed";
    turn.completedAt = turn.updatedAt = Date.now();
    turn.error = {
      code: "execution_failed",
      message: entry.pi ? "pi execution failed" : e.message,
      retryable: true,
    };
    entry.value.status = "error";
    delete entry.value.activeTurn;
    await emit("event.turnUpserted", { turn });
    await emit("event.conversationUpserted", { conversation: entry.value });
  }
}
async function business(method, p) {
  if (method === "provider.initialize") {
    if (p.protocol.minVersion > p.protocol.version)
      fail("invalid protocol interval", "invalid_protocol_range");
    const selectedVersion = Math.min(
      p.protocol.version,
      plugin.protocol.version,
    );
    if (
      selectedVersion <
      Math.max(p.protocol.minVersion, plugin.protocol.minVersion)
    )
      fail("no common business protocol version", "protocol_version_mismatch");
    dataDirectory = p.directories?.data;
    initialized = true;
    return { protocol: { ...plugin.protocol, selectedVersion }, plugin };
  }
  if (!initialized) fail("initialize required", "not_initialized");
  switch (method) {
    case "provider.describe":
      await sleep(controls.describeDelayMs);
      return { plugin };
    case "provider.ping": {
      const signature = JSON.stringify(
        p.clients.connections.map((c) => ({
          connectionId: c.connectionId,
          clientId: c.clientId,
        })),
      );
      if (
        pingState &&
        (p.hostSessionId !== pingState.session ||
          p.sequence <= pingState.sequence ||
          p.clients.revision < pingState.revision ||
          (p.clients.revision === pingState.revision &&
            signature !== pingState.signature))
      )
        fail("invalid heartbeat state");
      pingState = {
        session: p.hostSessionId,
        sequence: p.sequence,
        revision: p.clients.revision,
        signature,
      };
      return { sequence: p.sequence, clientsRevision: p.clients.revision };
    }
    case "instance.create": {
      if (p.instanceKind !== kind) fail("unsupported instance kind");
      if (p.route.providerPluginId !== pluginId) fail("plugin route mismatch");
      let old = instances.get(p.route.providerInstanceId);
      if (old) return { instance: old.value };
      const value = {
        route: p.route,
        pluginId,
        instanceKind: p.instanceKind,
        displayName: p.displayName,
        harness:
          backend === "pi"
            ? { id: "independent-pi-rpc", displayName: "Pi coding agent RPC" }
            : { id: "independent-js", displayName: "Independent JS Echo" },
        status: "created",
        capabilities,
      };
      instances.set(p.route.providerInstanceId, {
        value,
        settings: p.settings,
      });
      return { instance: value };
    }
    case "instance.start":
    case "instance.stop": {
      const inst = instance(p.route);
      if (method === "instance.start" && backend === "pi")
        for (const entry of conversations.values())
          if (
            entry.value.resource.providerId === p.route.providerInstanceId &&
            entry.pi?.closed
          ) {
            entry.pi = new PiSession({ cwd: entry.cwd });
            const state = await entry.pi.state();
            if (state.model) entry.value.model = state.model;
          }
      inst.value.status = method === "instance.start" ? "ready" : "stopped";
      if (method === "instance.stop")
        for (const entry of conversations.values())
          if (entry.value.resource.providerId === p.route.providerInstanceId) {
            entry.pi?.close();
            entry.value.status = "idle";
            delete entry.value.activeTurn;
          }
      if (method === "instance.stop")
        for (const turn of turns.values())
          if (
            turn.resource.providerId === p.route.providerInstanceId &&
            turn.status === "running"
          )
            turn.status = "interrupted";
      return { instance: inst.value };
    }
    case "instance.destroy":
      instance(p.route);
      for (const entry of conversations.values())
        if (entry.value.resource.providerId === p.route.providerInstanceId)
          entry.pi?.close();
      instances.delete(p.route.providerInstanceId);
      return { destroyed: true };
    case "instance.capabilities":
      instance(p.route);
      return { capabilities };
    case "runtime.getInstalled":
      return {
        selectionRequired: false,
        harnessList: [],
        selected: null,
        scanning: false,
      };
    case "conversation.create": {
      instance(p.route);
      const selection = normalizeSelection(capabilities.turnSend, {
        accessModeId: p.permissionLevel,
        ...(p.model !== undefined
          ? { model: { kind: "flat", modelId: p.model } }
          : {}),
        ...(p.reasoningEffort !== undefined
          ? { reasoningEffortId: p.reasoningEffort }
          : {}),
      });
      const native = id("conversation");
      const value = {
        resource: routed(p.route, native),
        project: p.project ? routed(p.route, p.project.nativeResourceId) : null,
        title: p.title || "Echo conversation",
        status: "idle",
        permissionLevel: p.permissionLevel,
        createdAt: Date.now(),
        updatedAt: Date.now(),
        selection,
      };
      if (selection.model) value.model = selection.model.modelId;
      if (p.workspaceRoot) value.workspaceRoot = p.workspaceRoot;
      const entry = { value, items: [] };
      if (backend === "pi") {
        const cwd = p.workspaceRoot || dataDirectory;
        if (!cwd)
          fail("pi requires workspaceRoot or initialization data directory");
        fs.mkdirSync(cwd, { recursive: true });
        entry.cwd = cwd;
        entry.pi = new PiSession({ cwd });
        try {
          const state = await entry.pi.state();
          if (state.model) value.model = state.model;
        } catch (e) {
          entry.pi.close();
          throw e;
        }
      }
      conversations.set(`${p.route.providerInstanceId}:${native}`, entry);
      return { conversation: value };
    }
    case "conversation.list": {
      instance(p.route);
      if (p.query || p.readerScope) fail("unsupported query", "unsupported");
      let values = [...conversations.values()]
        .map((v) => v.value)
        .filter((v) => v.resource.providerId === p.route.providerInstanceId)
        .filter(
          (v) =>
            p.projectFilter.kind === "all" ||
            (p.projectFilter.kind === "standalone"
              ? v.project === null
              : v.project?.nativeResourceId ===
                p.projectFilter.project.nativeResourceId),
        );
      const binding = JSON.stringify([p.route, p.projectFilter]);
      let offset = 0;
      if (p.cursor) {
        try {
          const c = JSON.parse(Buffer.from(p.cursor, "base64url"));
          if (c.binding !== binding) throw Error();
          offset = c.offset;
        } catch {
          fail("invalid cursor", "invalid_cursor");
        }
      }
      const limit = p.limit || 20;
      const pageInfo = {};
      if (offset + limit < values.length)
        pageInfo.nextCursor = Buffer.from(
          JSON.stringify({ binding, offset: offset + limit }),
        ).toString("base64url");
      return { conversations: values.slice(offset, offset + limit), pageInfo };
    }
    case "conversation.get": {
      const { entry } = conversation(p.conversation);
      const turnIds = entry.items
        .slice()
        .reverse()
        .map((i) => i.turn.nativeResourceId)
        .filter((v, i, a) => a.indexOf(v) === i);
      const binding = JSON.stringify(
        [
          "deviceId",
          "providerPluginId",
          "providerInstanceId",
          "nativeResourceId",
        ].map((key) => p.conversation[key]),
      );
      let offset = 0,
        snapshot = turnIds;
      if (p.cursor) {
        try {
          const cursor = JSON.parse(Buffer.from(p.cursor, "base64url"));
          if (
            cursor.binding !== binding ||
            !Array.isArray(cursor.turnIds) ||
            !Number.isSafeInteger(cursor.offset) ||
            cursor.offset < 0
          )
            throw Error();
          offset = cursor.offset;
          snapshot = cursor.turnIds;
        } catch {
          fail("invalid history cursor", "invalid_cursor");
        }
      }
      const limit = p.limit || 100,
        selected = new Set(snapshot.slice(offset, offset + limit)),
        pageInfo = {};
      if (offset + limit < snapshot.length)
        pageInfo.nextCursor = Buffer.from(
          JSON.stringify({
            binding,
            offset: offset + limit,
            turnIds: snapshot,
          }),
        ).toString("base64url");
      return {
        conversation: entry.value,
        items: entry.items.filter((i) => selected.has(i.turn.nativeResourceId)),
        pageInfo,
      };
    }
    case "conversation.acquireInteraction": {
      const { entry } = conversation(p.conversation);
      return {
        selection: normalizeSelection(
          capabilities.turnSend,
          entry.value.selection,
        ),
      };
    }
    case "conversation.releaseInteraction":
      conversation(p.conversation);
      return { released: true, scope: "providerInstance" };
    case "instance.releaseControl":
      instance(p.route);
      return { outcome: "unsupported" };
    case "turn.start": {
      const { entry, inst } = conversation(p.conversation);
      if (inst.value.status !== "ready")
        fail("instance is not ready", "unavailable");
      if (p.capabilityRevision !== capabilities.revision)
        fail("capability revision mismatch", "capability_changed");
      const selection = normalizeSelection(capabilities.turnSend, p.selection);
      const key = `${p.conversation.providerInstanceId}:${p.clientRequestId}`,
        signature = JSON.stringify({
          conversation: [
            "deviceId",
            "providerPluginId",
            "providerInstanceId",
            "nativeResourceId",
          ].map((key) => p.conversation[key]),
          clientRequestId: p.clientRequestId,
          capabilityRevision: p.capabilityRevision,
          input: { kind: p.input.kind, text: p.input.text },
          selection,
        });
      if (accepted.has(key)) {
        const old = accepted.get(key);
        if (old.signature !== signature)
          fail("clientRequestId reused with changed request");
        return old.result;
      }
      if (entry.value.activeTurn) fail("conversation busy", "busy");
      const turn = {
        resource: routed(inst.value.route, id("turn")),
        conversation: entry.value.resource,
        status: "running",
        startedAt: Date.now(),
        updatedAt: Date.now(),
      };
      const userItem = item(
        inst.value.route,
        entry.value,
        turn,
        "user",
        p.input.text,
        id("user"),
      );
      entry.items.push(userItem);
      turns.set(
        `${p.conversation.providerInstanceId}:${turn.resource.nativeResourceId}`,
        turn,
      );
      entry.value.activeTurn = turn;
      entry.value.status = "running";
      entry.value.selection = selection;
      const result = {
        accepted: true,
        turn: { ...turn },
        userItem,
        effectiveSelection: selection,
      };
      accepted.set(key, { signature, result });
      setTimeout(
        () => runTurn(entry, turn, p.input.text, inst).catch(fatal),
        0,
      );
      return result;
    }
    case "turn.interrupt": {
      const { entry } = conversation(p.conversation);
      instance(p.turn);
      const turn = turns.get(
        `${p.turn.providerInstanceId}:${p.turn.nativeResourceId}`,
      );
      if (
        !turn ||
        turn.conversation.nativeResourceId !== p.conversation.nativeResourceId
      )
        fail("turn not found", "not_found");
      if (turn.status === "running") {
        turn.status = "interrupted";
        if (entry.pi) await entry.pi.abort();
        turn.completedAt = turn.updatedAt = Date.now();
        entry.value.status = "idle";
        delete entry.value.activeTurn;
        setTimeout(() => emit("event.turnUpserted", { turn }).catch(fatal), 0);
      }
      return { turn };
    }
    case "event.subscribe":
      return {
        subscriptionId: p.subscriptionId,
        message: "ordered events enabled",
      };
    case "event.unsubscribe":
      return { subscriptionId: p.subscriptionId };
    case "provider.shutdown":
      for (const entry of conversations.values()) entry.pi?.close();
      closing = true;
      return { accepted: true };
    default:
      fail("unsupported method", "unsupported");
  }
}
async function handle(req, reply = send) {
  let response;
  try {
    if (
      req?.jsonrpc !== "2.0" ||
      typeof req.id !== "string" ||
      typeof req.method !== "string"
    )
      fail("invalid JSON-RPC request");
    const validate = requestValidators.get(req.method);
    if (!validate) fail("method not found", "method_not_found");
    if (!validate(req.params ?? {}))
      fail(`invalid params: ${JSON.stringify(validate.errors)}`);
    const result = await business(req.method, req.params ?? {});
    const check = responseValidators.get(req.method);
    if (!check(result))
      throw Error(`invalid generated result: ${JSON.stringify(check.errors)}`);
    response = { jsonrpc: "2.0", id: req.id, result };
  } catch (e) {
    response = {
      jsonrpc: "2.0",
      id: req?.id ?? "",
      error: {
        code: e.protocolCode === "method_not_found" ? -32601 : -32000,
        message: e.message,
        data: {
          code: e.protocolCode || "internal_error",
          message: e.message,
          retryable: false,
        },
      },
    };
  }
  await reply(response);
  if (req?.method === "provider.shutdown" && !response.error) {
    await writeChain;
    process.exit(0);
  }
}
async function serial() {
  send = async (obj) => {
    if (profile === "stdio-jsonrpc-v1") {
      const buf = Buffer.from(JSON.stringify(obj) + "\n");
      if (buf.length > MAX_ENCODED) throw Error("JSONL output too large");
      await writeBuffer(buf);
    } else await writeBuffer((await frame(obj)).bytes);
  };
  let buf = Buffer.alloc(0);
  const decodes = new Set();
  for await (const chunk of process.stdin) {
    if (buf.length + chunk.length > MAX_ENCODED * 2)
      throw Error("input buffer oversized");
    buf = Buffer.concat([buf, chunk]);
    while (buf.length) {
      let size;
      if (profile === "stdio-jsonrpc-v1") {
        const pos = buf.indexOf(10);
        if (pos < 0) {
          if (buf.length >= MAX_ENCODED) throw Error("JSONL input oversized");
          break;
        }
        size = pos + 1;
        if (size > MAX_ENCODED) throw Error("JSONL input oversized");
        const raw = buf.subarray(0, pos);
        buf = buf.subarray(size);
        const req = JSON.parse(
          new TextDecoder("utf-8", { fatal: true }).decode(raw),
        );
        void handle(req);
        continue;
      }
      if (buf.length < 10) break;
      if (
        buf.toString("ascii", 0, 4) !== "CPRF" ||
        buf[4] !== 1 ||
        buf[5] !== 1
      )
        throw Error("invalid zstd CPRF header");
      size = buf.readUInt32BE(6) + 10;
      if (size > MAX_ENCODED) throw Error("CPRF input oversized");
      if (buf.length < size) break;
      const message = buf.subarray(0, size);
      buf = buf.subarray(size);
      const decode = unframe(message).then((req) => {
        void handle(req);
      });
      decodes.add(decode);
      decode.finally(() => decodes.delete(decode)).catch(fatal);
    }
  }
  if (buf.length) throw Error("truncated EOF record");
  await Promise.all(decodes);
  closing = true;
  for (const entry of conversations.values()) entry.pi?.close();
  await writeChain;
  process.exit(0);
}
if (profile === "stdio-jsonrpc-v1" || profile === "stdio-codepet-zstd-v1")
  serial().catch(fatal);
else if (profile === "stdio-codepet-mux-v1")
  import("./mux.mjs")
    .then((m) =>
      m.start({
        handle,
        frame,
        unframe,
        writeBuffer,
        controls,
        setSend: (f) => {
          send = f;
        },
      }),
    )
    .catch(fatal);
else fatal(Error("unsupported transport"));
