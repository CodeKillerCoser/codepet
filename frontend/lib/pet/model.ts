import type { ProviderConnectionState } from '../providerRuntimes';
import { writable } from 'svelte/store';
import { petSnapshot, type PetTask, type PetSource } from '../petGateway';

export interface PetData {
  tasks: PetTask[];
  sources: PetSource[];
  providers: ProviderConnectionState[];
  error: string;
}
const live = (task: PetTask) => ['running', 'waiting-approval', 'waiting-input'].includes(task.status);

// One subscription and one dismissal history for both concrete views.
export function createPetData(onNewTask: () => void = () => {}) {
  let current: PetData = { tasks: [], sources: [], providers: [], error: '' };
  const state = writable(current);
  const dismissed = new Map<string, number>();
  let busy = false, disposed = false;
  const publish = (next: PetData) => { current = next; state.set(next); };
  return {
    subscribe: state.subscribe,
    setProviders(providers: ProviderConnectionState[]) {
      if (!disposed) publish({ ...current, providers: providers.map(provider => ({ ...provider, providerId: provider.providerId.replace(/^dev\.codepet\./, '') })) });
    },
    async refresh() {
      if (busy || disposed) return;
      busy = true;
      try {
        const snapshot = await petSnapshot();
        if (disposed) return;
        const tasks = snapshot.tasks.filter(task => (dismissed.get(task.id) ?? -1) < task.updatedAt)
          .sort((a, b) => b.updatedAt - a.updatedAt);
        const previousLive = new Set(current.tasks.filter(live).map(task => task.id));
        if (tasks.some(task => live(task) && !previousLive.has(task.id))) onNewTask();
        publish({ ...current, tasks, sources: snapshot.sources ?? [], error: '' });
      } catch (error) {
        if (!disposed) publish({ ...current, error: String(error), tasks: current.tasks.map(task => live(task) ? { ...task, status: 'unknown' } : task) });
      } finally { busy = false; }
    },
    dismiss(task: PetTask) {
      dismissed.set(task.id, task.updatedAt);
      publish({ ...current, tasks: current.tasks.filter(item => item.id !== task.id) });
    },
    clearCompleted() {
      current.tasks.filter(task => task.status === 'completed').forEach(task => dismissed.set(task.id, task.updatedAt));
      publish({ ...current, tasks: current.tasks.filter(task => task.status !== 'completed') });
    },
    dispose() { disposed = true; },
  };
}
