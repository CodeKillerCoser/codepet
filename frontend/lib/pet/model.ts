import type { ProviderConnectionState } from '../providerRuntimes';
import { writable } from 'svelte/store';
import { noticeList, type Notice, type NoticeSource } from '../noticeGateway';

export interface PetData {
  notices: Notice[];
  sources: NoticeSource[];
  providers: ProviderConnectionState[];
  error: string;
}

// Both surfaces consume standard Notices and share local dismissal history.
export function createPetData(onNewNotice: () => void = () => {}) {
  let current: PetData = { notices: [], sources: [], providers: [], error: '' };
  const state = writable(current);
  const entries = new Map<string, Notice>();
  const dismissed = new Map<string, number>();
  let busy = false, disposed = false, providersKnown = false;
  const publish = (next: PetData) => { current = next; state.set(next); };
  const registered = (pluginId: string) => !providersKnown || current.providers.some(p => p.providerId === pluginId);
  function pruneOwners(owners: Set<string>) {
    for (const [id, entry] of entries) {
      if (!owners.has(entry.pluginId)) { entries.delete(id); dismissed.delete(id); }
    }
  }
  function merge(notices: Notice[]) {
    let isNew = false;
    for (const notice of notices) {
      if (!registered(notice.pluginId)) continue;
      const previous = entries.get(notice.noticeId);
      if (previous && previous.revision >= notice.revision) continue;
      if (!previous && !notice.finished) isNew = true;
      entries.set(notice.noticeId, notice);
    }
    for (const old of [...entries.values()].sort((a, b) => b.updatedAt - a.updatedAt).slice(400)) {
      entries.delete(old.noticeId); dismissed.delete(old.noticeId);
    }
    const visible = [...entries.values()]
      .filter(notice => (dismissed.get(notice.noticeId) ?? -1) < notice.revision)
      .sort((a, b) => b.updatedAt - a.updatedAt).slice(0, 200);
    publish({ ...current, notices: visible, error: '' });
    if (isNew) onNewNotice();
  }
  return {
    subscribe: state.subscribe,
    setProviders(providers: ProviderConnectionState[]) {
      if (disposed) return;
      providersKnown = true;
      const owners = new Set(providers.map(p => p.providerId));
      pruneOwners(owners);
      publish({ ...current, providers,
        notices: current.notices.filter(n => owners.has(n.pluginId)),
        sources: current.sources.filter(s => owners.has(s.pluginId)).map(source => ({ ...source,
          connected: source.connected && providers.some(p => p.providerId === source.pluginId && p.connectionStatus === 'online'),
        })),
      });
    },
    apply(notice: Notice) { if (!disposed) merge([notice]); },
    async refresh() {
      if (busy || disposed) return;
      busy = true;
      try {
        const snapshot = await noticeList();
        if (disposed) return;
        const sources = snapshot.sources.filter(s => registered(s.pluginId)).map(source => ({ ...source,
          connected: source.connected && (!providersKnown || current.providers.some(p => p.providerId === source.pluginId && p.connectionStatus === 'online')),
        }));
        // A reconnect may temporarily have no reported observation descriptor.
        // The Registry determines uninstall; absence from ready sources does not.
        const owners = new Set(providersKnown ? current.providers.map(p => p.providerId) : sources.map(s => s.pluginId));
        pruneOwners(owners);
        publish({ ...current, sources });
        merge(snapshot.notices.filter(n => owners.has(n.pluginId)));
      } catch (error) {
        if (!disposed) publish({ ...current, error: String(error) });
      } finally { busy = false; }
    },
    dismiss(notice: Notice) {
      dismissed.set(notice.noticeId, notice.revision);
      publish({ ...current, notices: current.notices.filter(n => n.noticeId !== notice.noticeId) });
    },
    clearCompleted() {
      for (const notice of current.notices) if (notice.finished) dismissed.set(notice.noticeId, notice.revision);
      publish({ ...current, notices: current.notices.filter(n => !n.finished) });
    },
    dispose() { disposed = true; entries.clear(); dismissed.clear(); },
  };
}
