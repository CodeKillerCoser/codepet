import { expect, it, vi, afterEach } from 'vitest';
import { get } from 'svelte/store';
import { noticeList, type Notice } from '../noticeGateway';
import { notice,source } from '../noticeTestFixtures';
import { createPetData } from './model';
vi.mock('../noticeGateway',()=>({noticeList:vi.fn()}));
afterEach(()=>vi.resetAllMocks());
function snapshot(notices:Notice[]){vi.mocked(noticeList).mockResolvedValue({notices,sources:[source],hasMore:false});}
it('merges delta events and snapshots by revision without resurrecting old revisions',async()=>{
 const data=createPetData();snapshot([notice('old'),notice('new',2)]);await data.refresh();expect(get(data).notices.map(n=>n.noticeId)).toEqual(['new','old']);
 data.dismiss(get(data).notices[0]);await data.refresh();expect(get(data).notices.map(n=>n.noticeId)).toEqual(['old']);
 data.apply(notice('new',3));snapshot([notice('new',2)]);await data.refresh();expect(get(data).notices[0].revision).toBe(3);
});
it('keeps provider statuses on disconnect and clears only explicitly finished entries',async()=>{
 const data=createPetData();snapshot([notice('live'),notice('done',2,true)]);await data.refresh();vi.mocked(noticeList).mockRejectedValueOnce(new Error('offline'));await data.refresh();
 expect(get(data).notices.every(n=>n.status.value==='provider-defined')).toBe(true);expect(get(data).error).toContain('offline');data.clearCompleted();expect(get(data).notices.map(n=>n.noticeId)).toEqual(['live']);
});
it('ignores responses after disposal and keeps one snapshot in flight',async()=>{
 let finish!:(value:any)=>void;vi.mocked(noticeList).mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));const data=createPetData(),pending=data.refresh();await data.refresh();expect(noticeList).toHaveBeenCalledOnce();data.dispose();finish({notices:[notice('late')],sources:[source]});await pending;expect(get(data).notices).toEqual([]);
});
it('retains exact plugin identities and removes notices of uninstalled sources',async()=>{
 const data=createPetData(),providers=[{providerId:'observer',displayName:'Codex',connectionStatus:'online' as const,generation:1,instances:[]}];data.setProviders(providers);snapshot([notice('live')]);await data.refresh();expect(get(data).providers).toEqual(providers);
 data.setProviders([]);vi.mocked(noticeList).mockResolvedValue({notices:[],sources:[],hasMore:false});await data.refresh();expect(get(data).notices).toEqual([]);
});

it('removes unloaded providers immediately and ignores their stale in-flight snapshots',async()=>{const data=createPetData();data.setProviders([{providerId:'observer',displayName:'Observer',connectionStatus:'online',generation:1,instances:[]}]);snapshot([notice('live')]);await data.refresh();expect(get(data).notices).toHaveLength(1);data.setProviders([]);expect(get(data).notices).toEqual([]);await data.refresh();expect(get(data).notices).toEqual([]);expect(get(data).sources).toEqual([]);});
it('does not revive dismissed entries while a registered source reconnects',async()=>{const data=createPetData();data.setProviders([{providerId:'observer',displayName:'Observer',connectionStatus:'online',generation:1,instances:[]}]);snapshot([notice('live')]);await data.refresh();data.dismiss(get(data).notices[0]);vi.mocked(noticeList).mockResolvedValue({notices:[],sources:[],hasMore:false});await data.refresh();snapshot([notice('live')]);await data.refresh();expect(get(data).notices).toEqual([]);});
