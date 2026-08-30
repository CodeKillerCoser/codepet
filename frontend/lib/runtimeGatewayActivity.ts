import type {
  Approval,
  Conversation,
  ProtocolEvent,
  Provider,
  TurnOutputDeltaEvent,
  TurnTask,
  TurnTaskStatus,
} from "./generated/runtimeGateway";
import type { PetEvent, PetEventKind, TaskStatus } from "./types";

export interface RuntimeGatewayProjectionResult {
  activities: PetEvent[];
  provider?: Provider;
}

export class RuntimeGatewayActivityProjection {
  private readonly providersById = new Map<string, Provider>();
  private readonly conversationsByKey = new Map<string, Conversation>();
  private readonly turnsByKey = new Map<string, TurnTask>();
  private readonly approvalsById = new Map<string, Approval>();
  private readonly announcedApprovalIds = new Set<string>();
  private readonly outputByKey = new Map<string, string>();

  replaceProviders(providers: Provider[]): void {
    this.providersById.clear();
    for (const provider of providers) {
      this.providersById.set(provider.id, provider);
    }
  }

  replaceConversations(conversations: Conversation[]): PetEvent[] {
    this.conversationsByKey.clear();
    this.turnsByKey.clear();
    this.approvalsById.clear();
    this.outputByKey.clear();

    const activities: PetEvent[] = [];
    for (const conversation of conversations) {
      this.rememberConversation(conversation);
      const activity = this.activityFromConversation(
        conversation,
        `gateway-snapshot:${conversation.providerId}:${conversation.id}:${conversation.updatedAt}`,
        false,
        conversation,
      );
      if (activity && activity.status !== "idle") {
        activities.push(activity);
      }
    }
    return activities;
  }

  applyEvent(event: ProtocolEvent): RuntimeGatewayProjectionResult {
    const eventId = `gateway-event:${event.eventSequence}:${event.event}`;
    switch (event.event) {
      case "provider.statusChanged":
        this.providersById.set(event.payload.provider.id, event.payload.provider);
        if (event.payload.provider.status !== "ready") {
          this.clearProviderState(event.payload.provider.id);
        }
        return { activities: [], provider: event.payload.provider };
      case "conversation.upserted": {
        const conversation = event.payload.conversation;
        this.rememberConversation(conversation);
        const activity = this.activityFromConversation(conversation, eventId, false, event);
        return { activities: activity ? [activity] : [] };
      }
      case "turn.upserted": {
        const turn = event.payload.turn;
        this.rememberTurn(turn);
        if (isTerminalTurnStatus(turn.status)) {
          this.removeApprovalsForTurn(turn);
        }
        const activity = this.activityFromTurn(turn, eventId, true, event);
        return { activities: activity ? [activity] : [] };
      }
      case "turn.outputDelta": {
        const activity = this.activityFromOutput(event.payload, eventId, event);
        return { activities: activity ? [activity] : [] };
      }
      case "approval.requested": {
        const approval = event.payload.approval;
        const shouldRing = !this.announcedApprovalIds.has(approval.id);
        this.announcedApprovalIds.add(approval.id);
        this.approvalsById.set(approval.id, approval);
        const activity = this.activityFromApproval(approval, eventId, shouldRing, event);
        return { activities: activity ? [activity] : [] };
      }
      case "approval.resolved": {
        const approval = event.payload.approval;
        this.announcedApprovalIds.delete(approval.id);
        this.approvalsById.delete(approval.id);
        const activity = this.activityFromResolvedApproval(approval, eventId, event);
        return { activities: activity ? [activity] : [] };
      }
      default:
        throw new Error(`Unsupported Runtime Gateway event: ${(event as { event: string }).event}`);
    }
  }

  providers(): Provider[] {
    return Array.from(this.providersById.values());
  }

  refreshActivityProvider(activity: PetEvent): PetEvent {
    const context = activity.runtimeGateway;
    if (!context) {
      return activity;
    }
    const provider = this.providersById.get(context.provider.id);
    if (!provider || provider === context.provider) {
      return activity;
    }
    return {
      ...activity,
      runtimeGateway: {
        ...context,
        provider,
      },
    };
  }

  private rememberConversation(conversation: Conversation): void {
    this.conversationsByKey.set(conversationKey(conversation.providerId, conversation.id), conversation);
    if (conversation.activeTurn) {
      this.rememberTurn(conversation.activeTurn);
    }
  }

  private rememberTurn(turn: TurnTask): void {
    this.turnsByKey.set(turnKey(turn.providerId, turn.conversationId, turn.id), turn);
  }

  private activityFromConversation(
    conversation: Conversation,
    eventId: string,
    shouldRing: boolean,
    raw: unknown,
  ): PetEvent | null {
    const provider = this.readyProvider(conversation.providerId);
    if (!provider) {
      return null;
    }
    if (conversation.activeTurn) {
      return this.activityFromTurn(conversation.activeTurn, eventId, shouldRing, raw);
    }

    const latestTurn = this.latestTurn(conversation.providerId, conversation.id);
    if (conversation.status === "idle" && latestTurn && isTerminalTurnStatus(latestTurn.status)) {
      return this.activityFromTurn(latestTurn, eventId, shouldRing, raw);
    }

    const mapped = conversationActivityStatus(conversation.status);
    if (!mapped) {
      return null;
    }
    return this.petEvent({
      id: eventId,
      provider,
      conversation,
      kind: mapped.kind,
      status: mapped.status,
      message: conversation.preview || mapped.message,
      shouldRing: shouldRing && mapped.status === "failed",
      createdAt: conversation.updatedAt,
      raw,
    });
  }

  private activityFromTurn(
    turn: TurnTask,
    eventId: string,
    shouldRing: boolean,
    raw: unknown,
  ): PetEvent | null {
    const provider = this.readyProvider(turn.providerId);
    if (!provider) {
      return null;
    }
    const conversation = this.conversationsByKey.get(conversationKey(turn.providerId, turn.conversationId));
    const approval = this.pendingApprovalForTurn(turn.providerId, turn.conversationId, turn.id);
    if (approval && !isTerminalTurnStatus(turn.status)) {
      return this.activityFromApproval(approval, eventId, shouldRing, raw);
    }

    const mapped = turnActivityStatus(turn.status);
    const output = this.latestOutputForTurn(turn);
    return this.petEvent({
      id: eventId,
      provider,
      conversation,
      conversationId: turn.conversationId,
      turn,
      kind: mapped.kind,
      status: mapped.status,
      message: turn.displaySummary || output || conversation?.preview || mapped.message,
      shouldRing: shouldRing && (mapped.status === "done" || mapped.status === "failed"),
      createdAt: turn.completedAt ?? turn.updatedAt,
      raw,
    });
  }

  private activityFromOutput(payload: TurnOutputDeltaEvent, eventId: string, raw: unknown): PetEvent | null {
    const turn = this.turnsByKey.get(turnKey(payload.providerId, payload.conversationId, payload.turnId));
    if (!turn || this.pendingApprovalForTurn(payload.providerId, payload.conversationId, payload.turnId)) {
      return null;
    }
    const outputKey = turnOutputKey(payload.providerId, payload.conversationId, payload.turnId, payload.outputId);
    const accumulated = `${this.outputByKey.get(outputKey) ?? ""}${payload.delta}`.slice(-1200);
    this.outputByKey.set(outputKey, accumulated);
    const provider = this.readyProvider(payload.providerId);
    if (!provider) {
      return null;
    }
    const conversation = this.conversationsByKey.get(conversationKey(payload.providerId, payload.conversationId));
    const mapped = turnActivityStatus(turn.status);
    return this.petEvent({
      id: eventId,
      provider,
      conversation,
      conversationId: payload.conversationId,
      turn,
      kind: "task-updated",
      status: mapped.status,
      message: accumulated.trim() || turn.displaySummary || conversation?.preview || mapped.message,
      shouldRing: false,
      createdAt: turn.updatedAt,
      raw,
    });
  }

  private activityFromApproval(
    approval: Approval,
    eventId: string,
    shouldRing: boolean,
    raw: unknown,
  ): PetEvent | null {
    const provider = this.readyProvider(approval.providerId);
    if (!provider) {
      return null;
    }
    const conversation = this.conversationsByKey.get(conversationKey(approval.providerId, approval.conversationId));
    const turn = this.turnsByKey.get(turnKey(approval.providerId, approval.conversationId, approval.turnId));
    return this.petEvent({
      id: eventId,
      provider,
      conversation,
      conversationId: approval.conversationId,
      turn,
      approval,
      kind: "permission-requested",
      status: "waiting-approval",
      message: approval.description || approval.title,
      shouldRing: shouldRing && approval.status === "pending",
      createdAt: approval.requestedAt,
      raw,
    });
  }

  private activityFromResolvedApproval(approval: Approval, eventId: string, raw: unknown): PetEvent | null {
    const conversation = this.conversationsByKey.get(conversationKey(approval.providerId, approval.conversationId));
    const turn = this.turnsByKey.get(turnKey(approval.providerId, approval.conversationId, approval.turnId));
    if (turn) {
      return this.activityFromTurn(turn, eventId, false, raw);
    }
    return conversation
      ? this.activityFromConversation(conversation, eventId, false, raw)
      : null;
  }

  private petEvent(input: {
    id: string;
    provider: Provider;
    conversation?: Conversation;
    conversationId?: string;
    turn?: TurnTask;
    approval?: Approval;
    kind: PetEventKind;
    status: TaskStatus;
    message: string;
    shouldRing: boolean;
    createdAt?: number;
    raw: unknown;
  }): PetEvent {
    const conversationId = input.conversation?.id ?? input.conversationId ?? input.turn?.conversationId ?? input.approval?.conversationId ?? "";
    const createdAt = timestampToIso(input.createdAt);
    return {
      id: input.id,
      provider: input.provider.id,
      kind: input.kind,
      status: input.status,
      title: input.conversation?.title?.trim() || `${input.provider.displayName} 会话`,
      message: input.message,
      sessionId: conversationId,
      cwd: input.conversation?.workspaceRoot ?? null,
      toolName: input.approval?.kind ?? null,
      shouldRing: input.shouldRing,
      createdAt,
      endedAt: input.status === "done" || input.status === "failed" ? createdAt : null,
      raw: input.raw,
      source: null,
      runtimeGateway: {
        provider: input.provider,
        conversationId,
        turn: input.turn ?? null,
        approval: input.approval ?? null,
      },
    };
  }

  private readyProvider(providerId: string): Provider | null {
    const provider = this.providersById.get(providerId);
    return provider?.status === "ready" ? provider : null;
  }

  private latestTurn(providerId: string, conversationId: string): TurnTask | null {
    let latest: TurnTask | null = null;
    for (const turn of this.turnsByKey.values()) {
      if (turn.providerId !== providerId || turn.conversationId !== conversationId) {
        continue;
      }
      if (!latest || turn.updatedAt >= latest.updatedAt) {
        latest = turn;
      }
    }
    return latest;
  }

  private pendingApprovalForTurn(providerId: string, conversationId: string, turnId: string): Approval | null {
    let latest: Approval | null = null;
    for (const approval of this.approvalsById.values()) {
      if (
        approval.providerId === providerId &&
        approval.conversationId === conversationId &&
        approval.turnId === turnId &&
        approval.status === "pending" &&
        (!latest || approval.requestedAt >= latest.requestedAt)
      ) {
        latest = approval;
      }
    }
    return latest;
  }

  private latestOutputForTurn(turn: TurnTask): string {
    const prefix = `${turn.providerId}:${turn.conversationId}:${turn.id}:`;
    let latest = "";
    for (const [key, output] of this.outputByKey) {
      if (key.startsWith(prefix) && output.trim()) {
        latest = output.trim();
      }
    }
    return latest;
  }

  private removeApprovalsForTurn(turn: TurnTask): void {
    for (const [approvalId, approval] of this.approvalsById) {
      if (
        approval.providerId === turn.providerId &&
        approval.conversationId === turn.conversationId &&
        approval.turnId === turn.id
      ) {
        this.approvalsById.delete(approvalId);
      }
    }
  }

  private clearProviderState(providerId: string): void {
    const keyPrefix = `${providerId}:`;
    for (const key of this.conversationsByKey.keys()) {
      if (key.startsWith(keyPrefix)) {
        this.conversationsByKey.delete(key);
      }
    }
    for (const key of this.turnsByKey.keys()) {
      if (key.startsWith(keyPrefix)) {
        this.turnsByKey.delete(key);
      }
    }
    for (const [approvalId, approval] of this.approvalsById) {
      if (approval.providerId === providerId) {
        this.approvalsById.delete(approvalId);
      }
    }
    for (const key of this.outputByKey.keys()) {
      if (key.startsWith(keyPrefix)) {
        this.outputByKey.delete(key);
      }
    }
  }
}

function conversationKey(providerId: string, conversationId: string): string {
  return `${providerId}:${conversationId}`;
}

function turnKey(providerId: string, conversationId: string, turnId: string): string {
  return `${providerId}:${conversationId}:${turnId}`;
}

function turnOutputKey(providerId: string, conversationId: string, turnId: string, outputId: string): string {
  return `${turnKey(providerId, conversationId, turnId)}:${outputId}`;
}

function conversationActivityStatus(status: Conversation["status"]): { kind: PetEventKind; status: TaskStatus; message: string } | null {
  switch (status) {
    case "running":
      return { kind: "task-updated", status: "running", message: "会话正在运行" };
    case "waiting-approval":
      return { kind: "permission-requested", status: "waiting-approval", message: "等待审批或输入" };
    case "error":
      return { kind: "task-failed", status: "failed", message: "Provider 报告会话异常" };
    case "idle":
      return { kind: "task-updated", status: "idle", message: "会话空闲" };
    case "archived":
      return { kind: "task-updated", status: "idle", message: "会话已归档" };
    default:
      throw new Error(`Unsupported conversation status: ${status}`);
  }
}

function turnActivityStatus(status: TurnTaskStatus): { kind: PetEventKind; status: TaskStatus; message: string } {
  switch (status) {
    case "queued":
      return { kind: "task-started", status: "thinking", message: "任务已排队" };
    case "running":
      return { kind: "task-updated", status: "running", message: "任务正在执行" };
    case "waiting-approval":
      return { kind: "permission-requested", status: "waiting-approval", message: "等待审批或输入" };
    case "completed":
      return { kind: "task-completed", status: "done", message: "任务完成" };
    case "failed":
      return { kind: "task-failed", status: "failed", message: "任务失败" };
    case "interrupted":
      return { kind: "task-updated", status: "idle", message: "任务已停止" };
    default:
      throw new Error(`Unsupported turn status: ${status}`);
  }
}

function isTerminalTurnStatus(status: TurnTaskStatus): boolean {
  return status === "completed" || status === "failed" || status === "interrupted";
}

function timestampToIso(timestamp?: number): string {
  const date = new Date(timestamp ?? Date.now());
  if (!Number.isFinite(date.getTime())) {
    return new Date().toISOString();
  }
  return date.toISOString();
}
