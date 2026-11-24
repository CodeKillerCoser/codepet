// @vitest-environment jsdom
import {afterEach,expect,it,vi}from'vitest';
import{mount,unmount,tick}from'svelte';
import NoticeActions from './NoticeActions.svelte';
import{notice}from'./noticeTestFixtures';
import{noticeActionExecute}from'./noticeGateway';
vi.mock('./noticeGateway',()=>({noticeActionExecute:vi.fn()}));
let component:ReturnType<typeof mount>;
afterEach(async()=>{if(component)await unmount(component);document.body.innerHTML='';vi.resetAllMocks();});
const reply={id:'reply',kind:'reply' as const,label:'回复',enabled:true};
const activate={id:'open',kind:'activate' as const,label:'打开应用',enabled:true,target:{kind:'application' as const,applicationId:'com.test.app'}};
it('shows only enabled known actions, focuses input and sends the captured revision',async()=>{
 const n={...notice('n',7),actions:[reply,activate,{...reply,id:'disabled',enabled:false},{kind:'future-action',id:'future',label:'未来',enabled:true}]};
 component=mount(NoticeActions,{target:document.body,props:{notice:n}});await tick();expect(document.querySelectorAll('button')).toHaveLength(1);
 document.querySelector<HTMLButtonElement>('button')!.click();await tick();await tick();const textarea=document.querySelector('textarea')!;await vi.waitFor(()=>expect(document.activeElement).toBe(textarea));
 textarea.value='你好';textarea.dispatchEvent(new Event('input',{bubbles:true}));await tick();vi.mocked(noticeActionExecute).mockResolvedValue({executionId:'e',status:'unknown'});
 document.querySelector('form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));await tick();await tick();
 expect(noticeActionExecute).toHaveBeenCalledWith('n','reply',7,expect.any(String),{kind:'reply',text:'你好'});expect(document.body.textContent).toContain('结果待确认');
 document.querySelector<HTMLButtonElement>('button')!.click();expect(noticeActionExecute).toHaveBeenCalledOnce();expect(document.querySelector<HTMLButtonElement>('button')!.disabled).toBe(true);
});
it('requires permission choice and never presents action controls for offline sources',async()=>{
 const n={...notice('n'),actions:[{id:'permit',kind:'permission' as const,label:'授权',enabled:true,description:'本地测试授权',options:[{id:'deny',label:'拒绝'}]}]};
 component=mount(NoticeActions,{target:document.body,props:{notice:n,available:false}});await tick();expect(document.querySelector('button')).toBeNull();await unmount(component);
 component=mount(NoticeActions,{target:document.body,props:{notice:n}});await tick();document.querySelector<HTMLButtonElement>('button')!.click();await tick();expect(noticeActionExecute).not.toHaveBeenCalled();
 vi.mocked(noticeActionExecute).mockResolvedValue({executionId:'e',status:'succeeded'});[...document.querySelectorAll('button')].find(b=>b.textContent==='拒绝')!.click();await tick();expect(noticeActionExecute).toHaveBeenCalledWith('n','permit',1,expect.any(String),{kind:'permission',optionId:'deny'});
});
it('renders no action row for an activate-only notice',async()=>{
 const n={...notice('n',4),actions:[activate]};
 component=mount(NoticeActions,{target:document.body,props:{notice:n}});await tick();
 expect(document.querySelector('.notice-actions')).toBeNull();expect(document.querySelector('button')).toBeNull();expect(noticeActionExecute).not.toHaveBeenCalled();
});
