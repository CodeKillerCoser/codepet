import { describe, expect, it } from 'vitest';
import { groupActivities } from './activityGroups';
import type { PetTask } from './petGateway';

function task(id: string, status: PetTask['status']): PetTask {
  return { id, status, title: id, updatedAt: 1 };
}

describe('activity groups', () => {
  it('prioritizes attention, preserves order within groups and omits idle tasks', () => {
    const input = [task('done', 'completed'), task('run', 'running'), task('fail', 'failed'), task('wait', 'waiting-approval'), task('idle', 'idle')];
    expect(groupActivities(input).map(group => [group.id, group.activities.map(item => item.id)])).toEqual([
      ['attention', ['fail', 'wait']], ['active', ['run']], ['completed', ['done']],
    ]);
    expect(input.map(item => item.id)).toEqual(['done', 'run', 'fail', 'wait', 'idle']);
  });

  it('moves the current task between groups when its snapshot status changes', () => {
    for (const status of ['waiting-approval', 'running', 'completed'] as const) {
      const groups = groupActivities([task('task', status)]);
      expect(groups).toHaveLength(1);
      expect(groups[0].id).toBe(status === 'completed' ? 'completed' : status === 'running' ? 'active' : 'attention');
      expect(groups[0].activities.map(item => item.id)).toEqual(['task']);
    }
  });

  it('groups waiting-input, completed, interrupted and unknown snapshots', () => {
    const tasks = [task('input', 'waiting-input'), task('done', 'completed'), task('cancel', 'interrupted'), task('unknown', 'unknown')];
    expect(groupActivities(tasks).map(group => [group.id, group.activities.map(item => item.id)])).toEqual([
      ['attention', ['input', 'unknown']], ['completed', ['done', 'cancel']],
    ]);
  });

  it('omits empty groups', () => {
    expect(groupActivities([])).toEqual([]);
    expect(groupActivities([task('done', 'completed')]).map(group => group.id)).toEqual(['completed']);
  });
});
