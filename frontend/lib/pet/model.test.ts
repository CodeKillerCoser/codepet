import { expect, it, vi, afterEach } from 'vitest';
import { get } from 'svelte/store';
import { petSnapshot, type PetTask, type PetSnapshot } from '../petGateway';
import { createPetData } from './model';
vi.mock('../petGateway', () => ({ petSnapshot: vi.fn() }));
afterEach(() => vi.resetAllMocks());
const task = (id: string, updatedAt = 1, status: PetTask['status'] = 'running'): PetTask => ({ id, updatedAt, status, title: id });
function snapshot(tasks: PetTask[]) { vi.mocked(petSnapshot).mockResolvedValue({ tasks, sources: [] } as unknown as PetSnapshot); }
it('orders the latest status and keeps dismissals until that task has new activity', async () => {
  const data = createPetData(); snapshot([task('older'), task('latest', 2)]); await data.refresh();
  expect(get(data).tasks.map(t => t.id)).toEqual(['latest', 'older']);
  data.dismiss(get(data).tasks[0]); await data.refresh();
  expect(get(data).tasks.map(t => t.id)).toEqual(['older']);
  snapshot([task('latest', 3)]); await data.refresh();
  expect(get(data).tasks[0].id).toBe('latest');
});
it('marks live tasks unknown on disconnect without losing completed tasks', async () => {
  const data = createPetData(); snapshot([task('live'), task('done', 2, 'completed')]); await data.refresh();
  vi.mocked(petSnapshot).mockRejectedValueOnce(new Error('offline')); await data.refresh();
  expect(get(data).tasks.map(t => t.status)).toEqual(['completed', 'unknown']);
  expect(get(data).error).toContain('offline');
  data.clearCompleted(); expect(get(data).tasks.map(t => t.id)).toEqual(['live']);
});
it('shares a single in-flight request and ignores responses after disposal', async () => {
  let finish!: (snapshot: PetSnapshot) => void;
  vi.mocked(petSnapshot).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const data = createPetData(), pending = data.refresh(); await data.refresh();
  expect(petSnapshot).toHaveBeenCalledOnce(); data.dispose();
  finish({ tasks: [task('late')], sources: [] } as unknown as PetSnapshot); await pending;
  expect(get(data).tasks).toEqual([]);
});

it('preserves provider identities across task refresh and ignores updates after disposal', async () => {
  const data = createPetData();
  const providers = [{ providerId: 'codex', displayName: 'Codex', icon: 'https://example.com/codex.png', connectionStatus: 'online' as const, generation: 1, instances: [] }];
  data.setProviders(providers.map(provider => ({ ...provider, providerId: 'dev.codepet.codex' }))); snapshot([task('live')]); await data.refresh();
  expect(get(data).providers).toEqual(providers);
  data.dismiss(get(data).tasks[0]); expect(get(data).providers).toEqual(providers);
  data.dispose(); data.setProviders([]); expect(get(data).providers).toEqual(providers);
});
