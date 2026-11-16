// Deterministic, account-free observation fixture. No external actions are sent.
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const error = (message, code = "invalid_notice_action") =>
  Object.assign(Error(message), { protocolCode: code });
export class NoticeFixture {
  constructor({ call, pluginId, env = process.env }) {
    this.call = call;
    this.pluginId = pluginId;
    this.env = env;
    this.enabled = env.NOTICE_ENABLED !== "0";
    this.closed = false;
    this.started = false;
    this.latest = null;
    this.executions = new Map();
    this.executionCount = 0;
    this.unknownUsed = false;
    this.state = "idle";
    this.waiters = [];
    this.creationKey =
      env.NOTICE_CREATION_KEY || "independent-notice-lifecycle-v1";
    this.sourceId = env.NOTICE_SOURCE_ID || "fixture-source";
    this.actions = [
      {
        kind: "reply",
        id: "reply",
        label: "Fixture reply",
        enabled: true,
        placeholder: "A local deterministic reply",
      },
      {
        kind: "permission",
        id: "permission",
        label: "Fixture permission",
        enabled: true,
        description: "Local fixture decision only",
        options: [
          { id: "once", label: "Allow once" },
          { id: "deny", label: "Deny" },
        ],
      },
      {
        kind: "activate",
        id: "activate",
        label: "Open Node.js",
        enabled: true,
        target: { kind: "uri", uri: "https://nodejs.org/" },
      },
      {
        kind: "reply",
        id: "disabled",
        label: "Disabled fixture action",
        enabled: false,
      },
    ];
  }
  start() {
    if (this.started || this.closed || this.env.NOTICE_AUTOPUBLISH === "0")
      return;
    this.started = true;
    this.task = this.publish().catch((e) => {
      this.state =
        e.protocolCode === "rpc_result_unknown"
          ? "unknown"
          : `failed:${e.protocolCode || "rpc_error"}`;
    });
  }
  async gate() {
    while (!this.enabled && !this.closed)
      await new Promise((resolve) => this.waiters.push(resolve));
    if (this.closed) throw error("fixture stopped", "fixture_stopped");
  }
  close() {
    this.closed = true;
    for (const resolve of this.waiters.splice(0)) resolve();
  }
  async publish() {
    await this.gate();
    this.state = "creating";
    this.latest = (
      await this.call("notice.create", {
        creationKey: this.creationKey,
        sourceId: this.sourceId,
        status: {
          value: this.env.NOTICE_INITIAL_STATUS || "fixture_queued_blue",
          label: "Queued by fixture",
        },
        content: {
          title: "Independent Notice fixture",
          progress: { text: "Created locally", percent: 0 },
        },
        actions: this.actions,
      })
    ).notice;
    if (this.latest.pluginId !== this.pluginId)
      throw error(
        "Host returned another plugin notice",
        "invalid_notice_response",
      );
    if (this.latest.finished) {
      this.state = "already-finished";
      return;
    }
    this.state = "created";
    await pause(Number(this.env.NOTICE_UPDATE_DELAY_MS || 50));
    await this.gate();
    this.latest = (
      await this.call("notice.update", {
        noticeId: this.latest.noticeId,
        operationId: `${this.creationKey}:update:1`,
        expectedRevision: this.latest.revision,
        status: {
          value: this.env.NOTICE_UPDATE_STATUS || "fixture_working_violet",
          label: "Working in fixture",
        },
        content: {
          title: "Independent Notice fixture updated",
          progress: { text: "Deterministic local progress", percent: 50 },
        },
      })
    ).notice;
    this.state = "updated";
    await pause(Number(this.env.NOTICE_FINISH_DELAY_MS || 50));
    await this.gate();
    this.latest = (
      await this.call("notice.finish", {
        noticeId: this.latest.noticeId,
        operationId: `${this.creationKey}:finish:1`,
        expectedRevision: this.latest.revision,
        status: {
          value: this.env.NOTICE_FINAL_STATUS || "fixture_done_copper",
          label: "Fixture finished",
        },
        content: {
          title: "Independent Notice fixture finished",
          progress: {
            text: "No account, message, or model was contacted",
            percent: 100,
          },
        },
      })
    ).notice;
    this.state = "finished";
  }
  observe({ enabled }) {
    this.enabled = enabled;
    if (enabled) {
      for (const resolve of this.waiters.splice(0)) resolve();
      this.start();
    }
    return {
      enabled,
      message: `fixture observation ${enabled ? "enabled" : "disabled"}; publish=${this.state}; executions=${this.executionCount}`,
    };
  }
  execute(params) {
    const { executionId, noticeId, actionId, expectedRevision, input } = params;
    const signature = JSON.stringify({
      noticeId,
      actionId,
      expectedRevision,
      input:
        input.kind === "reply"
          ? { kind: "reply", text: input.text }
          : input.kind === "permission"
            ? { kind: "permission", optionId: input.optionId }
            : { kind: input.kind },
    });
    const accepted = this.executions.get(executionId);
    if (accepted) {
      if (accepted.signature !== signature)
        throw error(
          "executionId reused with changed request",
          "execution_conflict",
        );
      return accepted.result;
    }
    if (!this.latest || noticeId !== this.latest.noticeId)
      throw error("unknown fixture notice", "notice_not_found");
    if (expectedRevision !== this.latest.revision)
      throw error("notice revision changed", "revision_conflict");
    const action = this.latest.actions.find((action) => action.id === actionId);
    if (!action || action.enabled !== true)
      throw error("action missing or disabled");
    if (action.kind === "activate")
      throw error(
        "activate belongs to the Host platform adapter",
        "host_action_required",
      );
    if (action.kind !== input.kind) throw error("action kind mismatch");
    if (
      action.kind === "permission" &&
      !action.options?.some((option) => option.id === input.optionId)
    )
      throw error("unsupported permission option");
    if (this.executions.size >= 1024)
      throw error("fixture execution capacity exceeded", "capacity_exceeded");
    const count = ++this.executionCount;
    const unknown = this.env.NOTICE_UNKNOWN_ONCE === "1" && !this.unknownUsed;
    if (unknown) this.unknownUsed = true;
    const result = {
      executionId,
      status: unknown ? "unknown" : "succeeded",
      message: unknown
        ? `Fixture result unknown; execution ${count} will not be repeated`
        : action.kind === "reply"
          ? `Fixture reply receipt ${count}: ${input.text}`
          : `Fixture permission receipt ${count}: ${input.optionId}`,
    };
    this.executions.set(executionId, { signature, result });
    return result;
  }
}
