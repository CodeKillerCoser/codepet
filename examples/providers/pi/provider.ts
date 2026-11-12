import { createAgentSession, DefaultResourceLoader, ModelRuntime, SessionManager, type AgentSession } from '@earendil-works/pi-coding-agent';
import { DatabaseSync } from 'node:sqlite';
import { randomUUID } from 'node:crypto';
import { mkdirSync } from 'node:fs';
import { dirname, isAbsolute, join } from 'node:path';
import { homedir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { ProviderError, serve, type Handlers } from './server.ts';
import type { Conversation, ConversationItem, MessageConversationItem, ToolConversationItem, ProviderCapabilities, ProviderDirectories, ProviderInstance,
  ProviderInstanceRoute, ProviderResourceId, RoutedResourceId, ProtocolEvent, TurnTask } from './types/codepet-provider-sdk/src/generated.ts';

const PLUGIN = 'dev.codepet.example.pi';
const descriptor = { pluginId: PLUGIN, displayName: 'Pi (TypeScript)', version: '0.1.0',
  defaultWorkspaceRoot: homedir(), supportedVersions: { minVersion: 1, maxVersion: 1 }, instanceKinds: ['pi'] };
const revision = 'pi-readonly-v1';
type Saved = { conversation: Conversation; items: ConversationItem[]; turns: TurnTask[]; sessionFile?: string; requests: Record<string, { turnId: string; fingerprint: string }> };
type Live = Saved & { session?: AgentSession; loading?: Promise<AgentSession>; running?: Promise<void>; starting?: boolean; unsubscribe?: () => void };

export function createProvider(publish: (event: ProtocolEvent) => Promise<void>) {
  let device = '';
  let directories: ProviderDirectories | undefined;
  let database: DatabaseSync | undefined;
  let models: ModelRuntime | undefined;
  const instances = new Map<string, ProviderInstance>();
  const conversations = new Map<string, Live>();
  let closed = false;
  let hostIdentity = '';
  const fail = (code: string, message: string): never => { throw new ProviderError(code, message); };
  const notify = (event: ProtocolEvent) => { void publish(event).catch((error) => { console.error(error); process.exitCode = 1; }); };
  function routeCheck(route: ProviderInstanceRoute) {
    if (!device || closed) fail('provider_not_initialized', 'Initialize Provider first');
    if (route.deviceId !== device || route.providerPluginId !== PLUGIN || !instances.has(route.providerInstanceId)) fail('unknown_instance', 'Unknown pi instance route');
    return instances.get(route.providerInstanceId)!;
  }
  function resource(route: ProviderInstanceRoute, id: string): RoutedResourceId { return { providerId: route.providerInstanceId, nativeResourceId: id }; }
  function wireResource(value: RoutedResourceId): ProviderResourceId {
    return { deviceId: device, providerPluginId: PLUGIN, providerInstanceId: value.providerId, nativeResourceId: value.nativeResourceId };
  }
  function lookup(id: ProviderResourceId): Live {
    routeCheck(id);
    const live = conversations.get(id.nativeResourceId);
    if (!live || live.conversation.resource.providerId !== id.providerInstanceId) fail('unknown_conversation', 'Unknown pi conversation');
    return live!;
  }
  function save(live: Live) {
    const saved: Saved = { conversation: live.conversation, items: live.items, turns: live.turns, sessionFile: live.session?.sessionManager.getSessionFile() ?? live.sessionFile, requests: live.requests };
    database!.prepare('INSERT OR REPLACE INTO pi_conversations (id, snapshot) VALUES (?, ?)').run(live.conversation.resource.nativeResourceId, JSON.stringify(saved));
  }
  function capabilities(): ProviderCapabilities {
    const available = models?.getAvailableSnapshot() ?? [];
    const selection = { accessMode: { options: [{ id: 'read-only', displayName: 'Read only' }], defaultId: 'read-only' },
      ...(available.length ? { modelCatalog: { kind: 'flat' as const, models: available.map((m) => ({ id: `${m.provider}/${m.id}`, displayName: m.name })) } } : {}) };
    return { revision, methods: ['conversation.list', 'conversation.get', 'conversation.create',
      'conversation.releaseInteraction', 'instance.releaseControl', 'turn.start', 'turn.interrupt'],
      extensions: [], turnSend: selection, conversationCreate: { supportsTitle: true, selection } };
  }
  function snapshot(instance: ProviderInstance, status: ProviderInstance['status']) {
    const previousStatus = instance.status;
    instance.status = status;
    notify({ jsonrpc: '2.0', method: 'event.instanceStatusChanged', params: { instance, previousStatus } });
    return { instance };
  }
  async function sessionFor(live: Live) {
    if (live.session) return live.session;
    if (live.loading) return live.loading;
    live.loading = loadSession(live);
    try { return await live.loading; } finally { delete live.loading; }
  }
  async function loadSession(live: Live) {
    const cwd = live.conversation.workspaceRoot!;
    const manager = live.sessionFile ? SessionManager.open(live.sessionFile) : SessionManager.create(cwd, join(directories!.data, 'sessions'));
    const loader = new DefaultResourceLoader({ cwd, agentDir: process.env.CODEPET_PI_AGENT_DIR ?? join(homedir(), '.pi/agent'),
      noExtensions: true, noSkills: true, noPromptTemplates: true, noThemes: true });
    await loader.reload();
    const { session } = await createAgentSession({ cwd, agentDir: process.env.CODEPET_PI_AGENT_DIR,
      modelRuntime: models!, sessionManager: manager, resourceLoader: loader, tools: ['read', 'grep', 'find', 'ls'],
      // No writes or shell execution in this validation Provider.
      customTools: [], excludeTools: ['bash', 'write', 'edit'] });
    live.session = session;
    save(live);
    return session;
  }
  function item(live: Live, turn: TurnTask, role: 'user' | 'assistant', text: string, status: MessageConversationItem['status']): MessageConversationItem {
    const id = `${turn.resource.nativeResourceId}:${role}`;
    return { resource: { ...turn.resource, nativeResourceId: id }, turn: turn.resource, conversation: live.conversation.resource,
      kind: 'message', status, role, contents: [{ contentId: `${id}:text`, kind: 'text', text }] };
  }
  function upsert(live: Live, value: ConversationItem) {
    const index = live.items.findIndex((other) => other.resource.nativeResourceId === value.resource.nativeResourceId);
    if (index < 0) live.items.push(value); else live.items[index] = value;
    notify({ jsonrpc: '2.0', method: 'event.conversationItemUpserted', params: { item: value } });
  }
  function turnChanged(live: Live, turn: TurnTask) {
    turn.updatedAt = Date.now(); live.conversation.updatedAt = turn.updatedAt;
    live.conversation.status = turn.status === 'running' ? 'running' : turn.status === 'failed' ? 'error' : 'idle';
    if (turn.status === 'running') live.conversation.activeTurn = turn; else delete live.conversation.activeTurn;
    notify({ jsonrpc: '2.0', method: 'event.turnUpserted', params: { turn } });
    notify({ jsonrpc: '2.0', method: 'event.conversationUpserted', params: { conversation: live.conversation } });
    save(live);
  }
  async function run(live: Live, turn: TurnTask, text: string) {
    let output = '';
    const session = live.session!;
    const stop = session.subscribe((event) => {
      if (event.type === 'tool_execution_start') {
        const value: ToolConversationItem = { resource: { ...turn.resource, nativeResourceId: `${turn.resource.nativeResourceId}:tool:${event.toolCallId}` },
          turn: turn.resource, conversation: live.conversation.resource, kind: 'tool', status: 'running', title: event.toolName,
          tool: { callId: event.toolCallId, name: event.toolName, category: event.toolName === 'read' ? 'read' : 'search', origin: { kind: 'builtin' },
            input: { kind: 'structured', value: event.args } } };
        upsert(live, value);
      }
      if (event.type === 'tool_execution_end') {
        const previous = live.items.find((value) => value.kind === 'tool' && value.tool.callId === event.toolCallId);
        if (previous?.kind === 'tool') {
          const value = structuredClone(previous);
          const content = [{ contentId: `${value.resource.nativeResourceId}:output`, kind: 'text' as const, text: JSON.stringify(event.result) }];
          value.status = event.isError ? 'failed' : 'completed';
          value.tool.outcome = event.isError ? { kind: 'failure', content, error: { message: 'Pi tool failed' } } : { kind: 'success', content };
          upsert(live, value);
        }
      }
      if (event.type === 'message_update' && event.assistantMessageEvent.type === 'text_delta') {
        const delta = event.assistantMessageEvent.delta;
        output += delta;
        const value = item(live, turn, 'assistant', output, 'running');
        notify({ jsonrpc: '2.0', method: 'event.turnOutputDelta', params: { turn: wireResource(turn.resource), conversation: wireResource(live.conversation.resource),
          itemId: value.resource.nativeResourceId, contentId: `${value.resource.nativeResourceId}:text`, kind: 'text', delta } });
      }
    });
    live.unsubscribe = stop;
    try {
      await session.prompt(text);
      const last = session.state.messages.at(-1);
      if (last?.role === 'assistant' && last.stopReason === 'error') throw new Error(last.errorMessage ?? 'Pi model request failed');
      if (turn.status !== 'interrupted') turn.status = 'completed';
    } catch (cause) {
      if (turn.status !== 'interrupted') {
        turn.status = 'failed'; turn.error = { code: 'pi_turn_failed', message: String(cause), retryable: true };
      }
    } finally {
      stop(); delete live.unsubscribe;
      turn.completedAt = Date.now();
      upsert(live, item(live, turn, 'assistant', output, turn.status === 'failed' ? 'failed' : turn.status === 'interrupted' ? 'interrupted' : 'completed'));
      turnChanged(live, turn); delete live.running;
    }
  }
  async function stopInstance(id: string) {
    for (const live of conversations.values()) if (live.conversation.resource.providerId === id) {
      const turn = live.conversation.activeTurn;
      if (turn) turn.status = 'interrupted';
      await live.session?.abort(); await live.running;
      live.session?.dispose(); delete live.session;
    }
  }
  const handlers: Handlers = {
    'provider.initialize': async (request) => {
      if (closed) fail('provider_shutdown', 'Provider is shut down');
      if (!request.hostClientId.trim() || !request.hostDeviceId.trim() || !request.hostVersion.trim() || request.supportedVersions.minVersion > 1 || request.supportedVersions.maxVersion < 1) fail('unsupported_host', 'Incompatible Host initialization');
      const identity = JSON.stringify([request.hostClientId, request.hostDeviceId]);
      if (hostIdentity && hostIdentity !== identity) fail('host_identity_changed', 'Host identity changed');
      if (!request.directories) fail('directories_required', 'Host must assign private storage');
      const dirs = request.directories!;
      if (![dirs.data, dirs.logs, dirs.databasePath].every(isAbsolute) || dirname(dirs.databasePath) !== dirs.data) fail('invalid_directories', 'Invalid private Provider directories');
      if (directories && JSON.stringify(directories) !== JSON.stringify(dirs)) fail('directories_changed', 'Private directories changed after initialization');
      if (!database) {
        mkdirSync(dirs.data, { recursive: true }); mkdirSync(dirs.logs, { recursive: true });
        const db = new DatabaseSync(dirs.databasePath);
        try {
        db.exec('PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS pi_conversations(id TEXT PRIMARY KEY, snapshot TEXT NOT NULL)');
        models = await ModelRuntime.create({ ...(process.env.CODEPET_PI_AGENT_DIR ? { authPath: join(process.env.CODEPET_PI_AGENT_DIR, 'auth.json'), modelsPath: join(process.env.CODEPET_PI_AGENT_DIR, 'models.json') } : {}), allowModelNetwork: false });
        for (const row of db.prepare('SELECT snapshot FROM pi_conversations').all()) {
          const saved: Saved = JSON.parse(String(row.snapshot));
          if (saved.conversation.status === 'running') {
            saved.conversation.status = 'error'; delete saved.conversation.activeTurn;
            for (const turn of saved.turns) if (turn.status === 'running') { turn.status = 'failed'; turn.error = { code: 'provider_restarted', message: 'Provider restarted during turn', retryable: true }; }
          }
          conversations.set(saved.conversation.resource.nativeResourceId, saved);
        }
        database = db; directories = dirs; device = request.hostDeviceId; hostIdentity = identity;
        console.error('Pi Provider initialized: private SQLite opened, pi SDK loaded');
        } catch (cause) { db.close(); throw cause; }
      }
      return { selectedVersion: 1, plugin: descriptor };
    },
    'provider.describe': () => ({ plugin: descriptor }),
    'runtime.getInstalled': () => {
      const runtime = { executablePath: fileURLToPath(new URL('./node_modules/@earendil-works/pi-coding-agent/dist/cli.js', import.meta.url)), version: '1.5.2', source: 'configured' as const };
      return { harnessList: [runtime], selected: runtime, scanning: false };
    },
    'instance.create': (request) => {
      if (request.route.deviceId !== device || request.route.providerPluginId !== PLUGIN || request.instanceKind !== 'pi') fail('invalid_instance', 'Invalid pi instance');
      let instance = instances.get(request.route.providerInstanceId);
      if (!instance) {
        instance = { route: request.route, pluginId: PLUGIN, instanceKind: 'pi', displayName: request.displayName ?? 'Pi',
          harness: { id: 'pi', displayName: 'Pi SDK', version: '1.5.2', executablePath: process.execPath }, status: 'created', capabilities: capabilities() };
        instances.set(request.route.providerInstanceId, instance);
      }
      return { instance };
    },
    'instance.start': (request) => snapshot(routeCheck(request.route), 'ready'),
    'instance.stop': async (request) => { const instance = routeCheck(request.route); await stopInstance(request.route.providerInstanceId); return snapshot(instance, 'stopped'); },
    'instance.destroy': async (request) => { routeCheck(request.route); await stopInstance(request.route.providerInstanceId); instances.delete(request.route.providerInstanceId); return { destroyed: true }; },
    'instance.capabilities': (request) => { routeCheck(request.route); return { capabilities: capabilities() }; },
    'instance.releaseControl': async (request) => { const instance = routeCheck(request.route); await stopInstance(request.route.providerInstanceId); snapshot(instance, 'ready'); return { outcome: 'released' }; },
    'conversation.releaseInteraction': (request) => { lookup(request.conversation); return { released: true, scope: 'providerInstance' }; },
    'conversation.acquireInteraction': async (request) => {
      const live = lookup(request.conversation); const session = await sessionFor(live);
      return { selection: { accessModeId: 'read-only', ...(session.model ? { model: { kind: 'flat', modelId: `${session.model.provider}/${session.model.id}` } } : {}) } };
    },
    'conversation.create': async (request) => {
      routeCheck(request.route);
      if (request.permissionLevel !== 'read-only') fail('unsupported_permission', 'This validation Provider supports read-only access');
      const workspaceRoot = request.workspaceRoot ?? homedir();
      if (!isAbsolute(workspaceRoot)) fail('invalid_workspace', 'Use an absolute workspace path');
      if (request.project) fail('unsupported_project', 'Pi example supports independent workspaces');
      const conversation: Conversation = { resource: resource(request.route, randomUUID()), project: null, title: request.title ?? 'Pi conversation',
        status: 'idle', permissionLevel: 'read-only', workspaceRoot, createdAt: Date.now(), updatedAt: Date.now() };
      const live: Live = { conversation, items: [], turns: [], requests: {} };
      conversations.set(conversation.resource.nativeResourceId, live); save(live);
      notify({ jsonrpc: '2.0', method: 'event.conversationUpserted', params: { conversation } });
      return { conversation };
    },
    'conversation.list': (request) => {
      routeCheck(request.route);
      if (request.query || request.readerScope) fail('unsupported_query', 'Query/read state not advertised');
      let values = [...conversations.values()].filter((live) => live.conversation.resource.providerId === request.route.providerInstanceId).map((live) => live.conversation).sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
      if (request.projectFilter.kind === 'project') values = [];
      const offset = request.cursor ? Number(request.cursor) : 0;
      if (!Number.isSafeInteger(offset) || offset < 0) fail('invalid_cursor', 'Invalid list cursor');
      const limit = request.limit ?? 50;
      return { conversations: values.slice(offset, offset + limit), pageInfo: offset + limit < values.length ? { nextCursor: String(offset + limit) } : {} };
    },
    'conversation.get': (request) => {
      const live = lookup(request.conversation);
      const before = request.cursor ? Number(request.cursor) : live.turns.length;
      if (!Number.isSafeInteger(before) || before < 0 || before > live.turns.length) fail('invalid_cursor', 'Invalid history cursor');
      const begin = Math.max(0, before - (request.limit ?? 20));
      const ids = new Set(live.turns.slice(begin, before).map((turn) => turn.resource.nativeResourceId));
      return { conversation: live.conversation, items: live.items.filter((item) => ids.has(item.turn.nativeResourceId)),
        pageInfo: begin > 0 ? { nextCursor: String(begin) } : {} };
    },
    'turn.start': async (request) => {
      const live = lookup(request.conversation);
      if (request.capabilityRevision !== revision) fail('stale_capability_revision', 'Refresh capabilities');
      const fingerprint = JSON.stringify(request);
      const old = live.requests[request.clientRequestId];
      if (old) {
        if (old.fingerprint !== fingerprint) fail('request_id_conflict', 'Request ID already used for different input');
        const turn = live.turns.find((v) => v.resource.nativeResourceId === old.turnId)!;
        return { accepted: true, turn, userItem: live.items.find((v) => v.turn.nativeResourceId === old.turnId && v.kind === 'message' && v.role === 'user') ?? null, effectiveSelection: live.conversation.selection ?? {} };
      }
      if (live.running || live.starting) fail('turn_already_running', 'Pi conversation already running');
      if (request.input.kind !== 'text') fail('unsupported_input', 'Only text input supported');
      if (request.selection.accessModeId && request.selection.accessModeId !== 'read-only') fail('unsupported_permission', 'Read-only access required');
      if (request.selection.reasoningEffortId) fail('unsupported_selection', 'Reasoning effort selection is not advertised');
      live.starting = true;
      try {
      const session = await sessionFor(live);
      if (request.selection.model) {
        if (request.selection.model.kind !== 'flat') fail('unsupported_model', 'Select a flat pi model');
        const id = request.selection.model.kind === 'flat' ? request.selection.model.modelId : '';
        const selected = models!.getAvailableSnapshot().find((model) => `${model.provider}/${model.id}` === id);
        if (!selected) fail('model_unavailable', 'Selected pi model is unavailable');
        await session.setModel(selected!);
      }
      if (!session.model) fail('model_unavailable', 'Configure pi credentials and a model before sending');
      const turn: TurnTask = { resource: resource(request.conversation, randomUUID()), conversation: live.conversation.resource, status: 'running', startedAt: Date.now() };
      live.turns.push(turn); live.requests[request.clientRequestId] = { turnId: turn.resource.nativeResourceId, fingerprint };
      live.conversation.selection = { accessModeId: 'read-only', model: { kind: 'flat', modelId: `${session.model!.provider}/${session.model!.id}` } };
      const userItem = item(live, turn, 'user', request.input.text, 'completed'); upsert(live, userItem); turnChanged(live, turn);
      // Acknowledge acceptance before running the model, and keep interrupt dispatch independent.
      live.running = new Promise<void>((resolve) => setImmediate(() => { void run(live, turn, request.input.text).finally(resolve); }));
      return { accepted: true, turn, userItem, effectiveSelection: live.conversation.selection };
      } finally { delete live.starting; }
    },
    'turn.interrupt': async (request) => {
      const live = lookup(request.conversation); const turn = live.turns.find((v) => v.resource.nativeResourceId === request.turn.nativeResourceId);
      if (!turn) fail('unknown_turn', 'Unknown pi turn');
      if (turn!.status === 'running') { turn!.status = 'interrupted'; await live.session?.abort(); await live.running; }
      return { turn: turn! };
    },
    'provider.shutdown': async () => {
      for (const id of instances.keys()) await stopInstance(id);
      closed = true; database?.close(); database = undefined;
      return { accepted: true };
    },
  };
  return handlers;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  let send: (event: ProtocolEvent) => Promise<void>;
  const server = await serve(createProvider((event) => send(event)));
  send = server.publish;
  await server.run();
}
