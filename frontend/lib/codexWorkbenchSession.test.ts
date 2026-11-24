import { get } from 'svelte/store';
import { describe, expect, it, vi } from 'vitest';
import { createWorkbenchSession } from './codexWorkbenchSession';
import type { Snapshot, Worktree } from './codexWorkbench';

const project = { id: 'repo', name: 'Repo', roots: ['/repo'] };
const row: Worktree = { id: 'w', projectId: 'repo', projectName: 'Repo', path: '/repo', main: true,
  conversations: [], git: { changes: [], upstream: true, ahead: 1, behind: 0 } };
const snapshot = (scanId: number): Snapshot => ({ scanId, projects: [project], worktrees: [row], warnings: [], scannedAt: 123 });
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function setup() {
  const api = vi.fn(async (op: string): Promise<any> => {
    if (op === 'projects') return { projects: [project], warnings: [] };
    if (op === 'preferences') return ['repo'];
    if (op === 'scan') return snapshot(1);
    return null;
  });
  return { api, session: createWorkbenchSession(api) };
}

describe('workbench application session', () => {
  it('keeps the snapshot and draft across page subscriptions without reloading or rescanning', async () => {
    const { api, session } = setup();
    await session.initialize();
    await session.scan();
    await session.selectProjects([]);
    session.page.query = 'feature';
    session.page.status = 'unpushed';
    session.subscribe(() => {})();
    await session.initialize();
    expect(get(session).snapshot).toEqual(snapshot(1));
    expect(get(session).draft).toEqual([]);
    expect(session.page).toMatchObject({ query: 'feature', status: 'unpushed' });
    expect(api.mock.calls.map(([op]) => op)).toEqual(['projects', 'preferences', 'scan', 'saveSelection']);
  });

  it('finishes a scan with no mounted page and restores its result on return', async () => {
    const { api, session } = setup();
    await session.initialize();
    const pending = deferred<Snapshot>();
    api.mockImplementationOnce(() => pending.promise);
    const unsubscribe = session.subscribe(() => {});
    const scanning = session.scan();
    unsubscribe();
    expect(get(session).loading).toBe(true);
    pending.resolve(snapshot(2));
    await scanning;
    expect(get(session)).toMatchObject({ loading: false, snapshot: snapshot(2) });
    expect(api.mock.calls.some(([op]) => op === 'cancel')).toBe(false);
  });

  it('keeps rechecked Git facts after the page leaves', async () => {
    const { api, session } = setup();
    await session.initialize();
    await session.scan();
    const pending = deferred<Worktree>();
    api.mockImplementationOnce(() => pending.promise);
    const checking = session.recheck(row);
    session.subscribe(() => {})();
    const updated = { ...row, git: { ...row.git, ahead: 0 } };
    pending.resolve(updated);
    await checking;
    expect(get(session).snapshot?.worktrees).toEqual([updated]);
    expect(get(session).rechecking.size).toBe(0);
  });

  it('discards superseded scan results and late recheck errors', async () => {
    const { api, session } = setup();
    await session.initialize();
    await session.scan();
    const checking = deferred<Worktree>(), old = deferred<Snapshot>(), latest = deferred<Snapshot>();
    api.mockImplementationOnce(() => checking.promise)
      .mockImplementationOnce(() => old.promise).mockImplementationOnce(() => latest.promise);
    const recheck = session.recheck(row), first = session.scan(), second = session.scan();
    expect(get(session).snapshot).toBeNull();
    latest.resolve(snapshot(3));
    await second;
    old.resolve(snapshot(2));
    checking.reject(new Error('expired'));
    await Promise.all([first, recheck]);
    expect(get(session)).toMatchObject({ loading: false, error: '', snapshot: snapshot(3) });
  });
});
