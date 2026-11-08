import { expect, it } from 'vitest';
import { menuStatus } from './menu';
import type { PetTask } from '../petGateway';
it('shares the displayed latest task and count without exposing long titles in the menu bar', () => {
 const tasks = [{ status: 'waiting-approval', title: 'private long prompt' }, { status: 'running' }] as PetTask[];
 expect(menuStatus(tasks, '')).toEqual({ label: '等待授权', count: 2 });
 expect(menuStatus(tasks, 'offline')).toEqual({ label: '连接中断', count: 2 });
 expect(menuStatus([], '')).toEqual({ label: '就绪', count: 0 });
});
