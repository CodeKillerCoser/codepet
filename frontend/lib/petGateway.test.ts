import { afterEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { petRequest, petSnapshot } from './petGateway';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
afterEach(() => vi.resetAllMocks());
describe('Pet Gateway boundary', () => {
  it('uses the Pet endpoint for read-only snapshots and source toggles', async () => {
    vi.mocked(invoke).mockImplementation(async (command, args: any) => {
      expect(command).toBe('pet_gateway_request');
      return { ...args.request, response: { status: 'ok', result: args.request.method === 'pet.snapshot'
        ? { snapshot: { revision: 1, tasks: [], approvals: [], generatedAt: 0 } }
        : { source: { id: 'codex', enabled: false } } } };
    });
    expect((await petSnapshot()).tasks).toEqual([]);
    expect((await petRequest('source.setEnabled', { sourceId: 'codex', enabled: false })).source.enabled).toBe(false);
  });
  it('rejects a response from a different request and surfaces Gateway errors', async () => {
    vi.mocked(invoke).mockImplementation(async (_command, args: any) => ({ ...args.request, id: 'other' }));
    await expect(petSnapshot()).rejects.toThrow('response mismatch');
    vi.mocked(invoke).mockImplementation(async (_command, args: any) => ({ ...args.request, response: { status: 'error', error: { message: 'Unavailable' } } }));
    await expect(petSnapshot()).rejects.toThrow('Unavailable');
  });
});
