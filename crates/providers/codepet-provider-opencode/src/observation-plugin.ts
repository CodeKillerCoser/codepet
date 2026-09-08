import { readFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';

const accepted = new Set(['session.created', 'session.updated', 'session.deleted',
  'session.status', 'session.idle', 'session.error', 'permission.asked', 'permission.replied',
  'question.asked', 'question.replied', 'question.rejected', 'message.updated']);

// OpenCode 1.18.25's public Plugin function returns Hooks. No beta SDK dependency.
export const CodePetObservation = async ({ client, directory }) => {
  const controller = new AbortController();
  const sessions = new Map();
  const queue = [];
  let draining = false;
  let gap = false;
  const metadata = (info) => info && ({ title: info.title, directory: info.directory, parentID: info.parentID });
  const remember = (id, info) => {
    sessions.delete(id);
    sessions.set(id, metadata(info));
    if (sessions.size > 120) sessions.delete(sessions.keys().next().value);
  };
  const drain = async () => {
    if (draining) return;
    draining = true;
    try {
      while (queue.length && !controller.signal.aborted) {
        const { event, observedAt, session: cached } = queue.shift();
        try {
          const endpoint = JSON.parse(await readFile(new URL('../codepet-observation/endpoint.json', import.meta.url), 'utf8'));
          const sessionID = event.properties?.sessionID ?? event.properties?.info?.sessionID ?? event.properties?.info?.id;
          // The stable SDK wraps session.get in { data }; the hook itself never awaits I/O.
          const session = cached ?? (sessionID && event.type !== 'session.deleted'
            ? metadata((await client.session.get({ path: { id: sessionID },
              signal: AbortSignal.any([controller.signal, AbortSignal.timeout(500)]) }).catch(() => undefined))?.data)
            : undefined);
          const previousGap = gap; gap = false;
          const body = JSON.stringify({ eventId: event.id ?? randomUUID(), payload: {
            ...event, session, cwd: session?.directory ?? directory,
            codepet_observed_at: observedAt, codepet_gap: previousGap,
          } });
          if (Buffer.byteLength(body) > 250000) { gap = true; continue; }
          const response = await fetch(endpoint.url, { method: 'POST',
            headers: { 'content-type': 'application/json', authorization: endpoint.token }, body,
            signal: AbortSignal.any([controller.signal, AbortSignal.timeout(1000)]) });
          if (!response.ok) gap = true;
        } catch { gap = true; /* Observation cannot change or block the original task. */ }
      }
    } finally { draining = false; }
  };
  return {
    event: async ({ event }) => {
      if (controller.signal.aborted || !accepted.has(event.type)) return;
      const sessionID = event.properties?.sessionID ?? event.properties?.info?.sessionID ?? event.properties?.info?.id;
      if (event.type !== 'message.updated' && event.properties?.info && sessionID) remember(sessionID, event.properties.info);
      if (queue.length >= 128) { gap = true; return; }
      if (event.type === 'message.updated') {
        const info = event.properties?.info;
        if (info?.role !== 'assistant' || !info.time?.completed || !info.tokens) return;
        event = { id: event.id, type: event.type, properties: { info: {
          id: info.id, sessionID: info.sessionID, role: info.role, time: info.time,
          modelID: info.modelID, providerID: info.providerID, tokens: info.tokens,
        } } };
      }
      queue.push({ event, observedAt: Date.now(), session: sessions.get(sessionID) });
      if (event.type === 'session.deleted') sessions.delete(sessionID);
      void drain();
    },
    dispose: async () => { controller.abort(); queue.length = 0; sessions.clear(); },
  };
};
