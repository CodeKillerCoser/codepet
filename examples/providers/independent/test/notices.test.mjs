import { test } from "node:test";
import assert from "node:assert/strict";
import { Peer, hostStore, waitUntil, DROP } from "./notice-peer.mjs";
import {
  requestValidators,
  responseValidators,
  validateNoticeAction,
} from "../contracts.mjs";
const profiles = [
  "stdio-jsonrpc-v1",
  "stdio-codepet-zstd-v1",
  "stdio-codepet-mux-v1",
];
for (const profile of profiles)
  test(`${profile}: bidirectional Notice lifecycle, control concurrency, observe and action receipts`, async (t) => {
    const store = hostStore();
    let releaseCreate;
    const blocked = new Promise((r) => (releaseCreate = r));
    const peer = await new Peer(
      profile,
      {
        NOTICE_UNKNOWN_ONCE: "1",
        NOTICE_UPDATE_DELAY_MS: "1",
        NOTICE_FINISH_DELAY_MS: "1",
      },
      async (message) => {
        if (message.method === "notice.create") await blocked;
        return store.handle(message);
      },
    ).connect();
    t.after(() => peer.close());
    const init = await peer.initialize();
    assert.deepEqual(init.protocol, {
      version: 4,
      minVersion: 4,
      selectedVersion: 4,
    });
    assert.deepEqual(init.plugin.features, ["observation"]);
    assert.deepEqual(init.plugin.instanceKinds, []);
    await assert.rejects(
      peer.rpc("notice.create", {
        creationKey: "wrong-direction",
        sourceId: "s",
        status: { value: "x", label: "X" },
        content: { title: "x" },
      }),
      (e) => e.rpcCode === -32601,
    );
    await assert.rejects(
      peer.rpc("instance.create", {
        route: {
          deviceId: "d",
          providerPluginId: "independent.notice.fixture",
          providerInstanceId: "no-agent",
        },
        instanceKind: "independent-echo",
        displayName: "no",
        settings: {},
      }),
      (e) => e.code === "unsupported",
    );
    await waitUntil(() => peer.requests.length === 1);
    assert.equal(peer.requests[0].method, "notice.create");
    const ping = await peer.rpc("provider.ping", {
      sequence: 1,
      hostSessionId: "fixture",
      clients: { revision: 1, connections: [] },
      instances: [],
    });
    assert.equal(
      ping.sequence,
      1,
      "heartbeat returns while notice.create is pending",
    );
    assert.equal(
      (await peer.rpc("notice.observe", { enabled: false })).enabled,
      false,
    );
    assert.deepEqual((await peer.rpc("provider.describe")).plugin, init.plugin);
    releaseCreate();
    await new Promise((r) => setTimeout(r, 40));
    assert.equal(
      peer.requests.length,
      1,
      "disabled observe prevents subsequent publishing",
    );
    await peer.rpc("notice.observe", { enabled: true });
    await waitUntil(() => peer.requests.length === 3);
    await waitUntil(() => [...store.notices.values()][0]?.finished);
    const notice = [...store.notices.values()][0];
    await waitUntil(() => peer.requests[2].method === "notice.finish");
    assert.deepEqual(
      peer.requests.map((m) => m.method),
      ["notice.create", "notice.update", "notice.finish"],
    );
    assert.equal(notice.revision, 3);
    assert.equal(notice.status.value, "fixture_done_copper");
    assert.equal(notice.finished, true);
    assert.ok(peer.requests[2].params.status);
    assert.equal(peer.requests[1].params.expectedRevision, 1);
    assert.equal(peer.requests[2].params.expectedRevision, 2);
    // Wait until the Provider has consumed the final Host response.
    await waitUntil(() => peer.requests.length === 3);
    let observe;
    for (let i = 0; i < 30; i++) {
      observe = await peer.rpc("notice.observe", { enabled: true });
      if (observe.message.includes("publish=finished")) break;
      await new Promise((r) => setTimeout(r, 5));
    }
    assert.match(observe.message, /publish=finished/);
    const reply = {
      noticeId: notice.noticeId,
      actionId: "reply",
      expectedRevision: 3,
      executionId: "reply-once",
      input: { kind: "reply", text: "本地🙂" },
    };
    const unknown = await peer.rpc("notice.action.execute", reply);
    assert.equal(unknown.status, "unknown");
    assert.deepEqual(
      await peer.rpc("notice.action.execute", reply),
      unknown,
      "unknown results are never re-executed",
    );
    await assert.rejects(
      peer.rpc("notice.action.execute", {
        ...reply,
        input: { kind: "reply", text: "changed" },
      }),
      (e) => e.code === "execution_conflict",
    );
    const permission = {
      noticeId: notice.noticeId,
      actionId: "permission",
      expectedRevision: 3,
      executionId: "permission-once",
      input: { kind: "permission", optionId: "once" },
    };
    const receipt = await peer.rpc("notice.action.execute", permission);
    assert.equal(receipt.status, "succeeded");
    assert.match(receipt.message, /receipt 2: once/);
    assert.deepEqual(
      await peer.rpc("notice.action.execute", permission),
      receipt,
    );
    for (const params of [
      {
        ...permission,
        executionId: "bad-option",
        input: { kind: "permission", optionId: "unknown" },
      },
      { ...reply, executionId: "disabled", actionId: "disabled" },
      { ...reply, executionId: "stale", expectedRevision: 2 },
      {
        ...reply,
        executionId: "host-activate",
        actionId: "activate",
        input: { kind: "activate" },
      },
    ])
      await assert.rejects(peer.rpc("notice.action.execute", params));
    const target = peer.requests[0].params.actions.find(
      (a) => a.kind === "activate",
    ).target;
    assert.deepEqual(target, { kind: "uri", uri: "https://nodejs.org/" });
    assert.equal(peer.requests.length, 3);
    assert.equal(peer.errors.length, 0);
    assert.equal(peer.stderr, "");
    await peer.rpc("provider.shutdown");
  });
for (const profile of profiles)
  test(`${profile}: outgoing unknown result is not automatically retried`, async (t) => {
    const peer = await new Peer(
      profile,
      { NOTICE_RPC_TIMEOUT_MS: "30" },
      async () => DROP,
    ).connect();
    t.after(() => peer.close());
    await peer.initialize();
    await waitUntil(() => peer.requests.length === 1);
    await new Promise((r) => setTimeout(r, 80));
    const state = await peer.rpc("notice.observe", { enabled: true });
    assert.match(state.message, /publish=unknown/);
    assert.equal(peer.requests.length, 1);
    assert.equal(
      (
        await peer.rpc("provider.ping", {
          sequence: 1,
          hostSessionId: "unknown",
          clients: { revision: 1, connections: [] },
          instances: [],
        })
      ).sequence,
      1,
    );
    await peer.rpc("provider.shutdown");
  });
test("Notice schemas validate free-form status and reject missing final status / malformed actions", () => {
  const finish = {
    noticeId: "n",
    operationId: "o",
    expectedRevision: 1,
    status: { value: "arbitrary_custom_state", label: "自定义" },
  };
  assert.equal(requestValidators.get("notice.finish")(finish), true);
  delete finish.status;
  assert.equal(requestValidators.get("notice.finish")(finish), false);
  assert.equal(
    requestValidators.get("notice.observe")({ enabled: "true" }),
    false,
  );
  assert.equal(
    responseValidators.get("notice.action.execute")({
      executionId: "e",
      status: "unknown",
    }),
    true,
  );
  assert.equal(
    responseValidators.get("notice.action.execute")({
      executionId: "e",
      status: "made-up",
    }),
    false,
  );
});

test("disabled observation requires explicit start; reconnect does not reopen a finished Notice", async (t) => {
  const store = hostStore();
  const first = await new Peer(
    "stdio-jsonrpc-v1",
    {
      NOTICE_ENABLED: "0",
      NOTICE_UPDATE_DELAY_MS: "1",
      NOTICE_FINISH_DELAY_MS: "1",
    },
    store.handle,
  ).connect();
  t.after(() => first.close());
  await first.initialize();
  await new Promise((r) => setTimeout(r, 30));
  assert.equal(first.requests.length, 0);
  assert.equal(
    (await first.rpc("notice.observe", { enabled: false })).enabled,
    false,
  );
  await first.rpc("notice.observe", { enabled: true });
  await waitUntil(() => [...store.notices.values()][0]?.finished);
  await first.rpc("provider.shutdown");
  const resumed = await new Peer(
    "stdio-jsonrpc-v1",
    {},
    store.handle,
  ).connect();
  t.after(() => resumed.close());
  await resumed.initialize();
  await waitUntil(() => resumed.requests.length === 1);
  await new Promise((r) => setTimeout(r, 30));
  assert.match(
    (await resumed.rpc("notice.observe", { enabled: true })).message,
    /publish=already-finished/,
  );
  assert.equal(resumed.requests.length, 1);
  assert.equal(store.notices.size, 1);
  await resumed.rpc("provider.shutdown");
});

test("Host rejects an update: fixture stops without fabricating finish", async (t) => {
  const store = hostStore();
  const peer = await new Peer(
    "stdio-jsonrpc-v1",
    { NOTICE_UPDATE_DELAY_MS: "1" },
    (message) => {
      if (message.method === "notice.update")
        throw Object.assign(Error("fixture conflict"), {
          code: "revision_conflict",
        });
      return store.handle(message);
    },
  ).connect();
  t.after(() => peer.close());
  await peer.initialize();
  await waitUntil(() => peer.requests.length === 2);
  await new Promise((r) => setTimeout(r, 25));
  assert.match(
    (await peer.rpc("notice.observe", { enabled: true })).message,
    /publish=failed:revision_conflict/,
  );
  assert.equal(peer.requests.length, 2);
  assert.equal([...store.notices.values()][0].finished, false);
  await peer.rpc("provider.shutdown");
});

test("final public standard Schema enforces update groups and enabled action configuration", () => {
  assert.equal(
    requestValidators.get("notice.update")({
      noticeId: "n",
      operationId: "o",
      expectedRevision: 1,
    }),
    false,
  );
  assert.equal(
    requestValidators.get("notice.update")({
      noticeId: "n",
      operationId: "o",
      expectedRevision: 1,
      actions: [],
    }),
    true,
  );
  assert.equal(
    validateNoticeAction({
      kind: "activate",
      id: "a",
      label: "Open",
      enabled: true,
    }),
    false,
  );
  assert.equal(
    validateNoticeAction({
      kind: "activate",
      id: "a",
      label: "Open",
      enabled: true,
      target: { kind: "uri", uri: "https://nodejs.org/" },
    }),
    true,
  );
  assert.equal(
    validateNoticeAction({
      kind: "activate",
      id: "a",
      label: "Open",
      enabled: false,
    }),
    true,
  );
  assert.equal(
    validateNoticeAction({
      kind: "permission",
      id: "p",
      label: "Decide",
      enabled: true,
    }),
    false,
  );
  assert.equal(
    validateNoticeAction({
      kind: "permission",
      id: "p",
      label: "Decide",
      enabled: true,
      description: "Local choice",
      options: [],
    }),
    false,
  );
  assert.equal(
    validateNoticeAction({
      kind: "permission",
      id: "p",
      label: "Decide",
      enabled: true,
      description: "Local choice",
      options: [{ id: "once", label: "Once" }],
    }),
    true,
  );
  assert.equal(
    validateNoticeAction({
      kind: "permission",
      id: "p",
      label: "Decide",
      enabled: false,
    }),
    true,
  );
});
