import { get, writable } from 'svelte/store';
import { workbenchRequest, type Project, type Snapshot, type SourceInfo, type Worktree } from './codexWorkbench';

interface ScanState {
  projects: Project[];
  draft: string[];
  snapshot: Snapshot | null;
  loading: boolean;
  catalogLoading: boolean;
  error: string;
  catalogWarnings: string[];
  sourceInfo?: SourceInfo;
  progress: { checked: number; total: number; phase: string };
  rechecking: Set<string>;
}

// This session shares the lifetime of the main webview and the Rust snapshot.
// Never restore scan IDs from disk: they authorize actions only in this process.
export function createWorkbenchSession(api = workbenchRequest) {
  const state = writable<ScanState>({
    projects: [], draft: [], snapshot: null, loading: false, catalogLoading: true,
    error: '', catalogWarnings: [], progress: { checked: 0, total: 0, phase: 'history' },
    rechecking: new Set(),
  });
  const patch = (changes: Partial<ScanState>) => state.update(value => ({ ...value, ...changes }));
  let initialization: Promise<void> | undefined;
  let saveQueue = Promise.resolve();
  let sequence = 0;
  const page = { taskListWidth:280, taskListCollapsed:false, taskLedger:{view:'board' as 'board'|'table',boardFits:undefined as boolean|undefined,sort:'updated_desc' as 'updated_desc'|'updated_asc'|'title',query:'',project:'',status:'all'}, query: '', filterProjects: [] as string[], related: false, period: 'all', date: '', status: 'all' };

  function persist() {
    const projectIds = [...get(state).draft];
    saveQueue = saveQueue.then(async () => { await api('saveSelection', { projectIds }); })
      .catch(e => patch({ error: String(e) }));
    return saveQueue;
  }

  async function loadProjects(initial = false) {
    patch({ catalogLoading: true, error: '' });
    try {
      const result = await api<{ projects: Project[]; warnings: string[]; sourceInfo?: SourceInfo }>('projects');
      const ids = initial ? await api<string[]>('preferences') : get(state).draft;
      patch({ projects: result.projects, catalogWarnings: result.warnings, sourceInfo: result.sourceInfo,
        draft: ids.filter(id => result.projects.some(p => p.id === id)) });
      if (!initial) await persist();
    } catch (e) { patch({ error: String(e) }); }
    finally { patch({ catalogLoading: false }); }
  }

  async function scan() {
    const seq = ++sequence;
    const projectIds = [...get(state).draft];
    // A new scan invalidates actions against the previous Rust snapshot.
    patch({ snapshot: null, loading: true, error: '', rechecking: new Set(), progress: { checked: 0, total: 0, phase: 'history' } });
    try {
      const snapshot = await api<Snapshot>('scan', { projectIds });
      if (seq !== sequence) return;
      patch({ snapshot, sourceInfo: snapshot.sourceInfo ?? get(state).sourceInfo });
    } catch (e) { if (seq === sequence) patch({ error: String(e) }); }
    finally { if (seq === sequence) patch({ loading: false }); }
  }

  async function recheck(row: Worktree) {
    const { snapshot, loading, rechecking } = get(state);
    if (!snapshot || loading || rechecking.has(row.id)) return;
    const seq = sequence;
    patch({ rechecking: new Set([...rechecking, row.id]) });
    try {
      const updated = await api<Worktree>('recheck', { scanId: snapshot.scanId, worktreeId: row.id });
      const current = get(state).snapshot;
      if (seq === sequence && current?.scanId === snapshot.scanId) {
        patch({ snapshot: { ...current, worktrees: current.worktrees.map(r => r.id === row.id ? updated : r) } });
      }
    } catch (e) { if (seq === sequence) patch({ error: String(e) }); }
    finally {
      if (seq === sequence) patch({ rechecking: new Set([...get(state).rechecking].filter(id => id !== row.id)) });
    }
  }

  return {
    subscribe: state.subscribe, page, patch, loadProjects, scan, recheck,
    initialize() { return initialization ??= loadProjects(true); },
    selectProjects(draft: string[]) { patch({ draft }); return persist(); },
  };
}

export const workbenchSession = createWorkbenchSession();
