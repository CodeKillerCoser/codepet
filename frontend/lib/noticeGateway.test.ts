import { expect,it,vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { noticeList,noticeSourceSetEnabled,noticeActionExecute } from './noticeGateway';
vi.mock('@tauri-apps/api/core',()=>({invoke:vi.fn()}));
it('uses the existing canonical Gateway bridge for snapshots, source control and actions',async()=>{
 vi.mocked(invoke).mockImplementation(async(command,args:any)=>{expect(command).toBe('codepet_gateway_request');return {jsonrpc:'2.0',id:args.request.id,result:{notices:[],sources:[],hasMore:false}};});
 await noticeList();await noticeSourceSetEnabled('observer',false);await noticeActionExecute('n','r',3,'execution-1',{kind:'reply',text:'你好'});
 expect(vi.mocked(invoke).mock.calls[2][1]).toMatchObject({request:{method:'notice.action.execute',params:{expectedRevision:3,executionId:'execution-1',input:{kind:'reply',text:'你好'}}}});
});
