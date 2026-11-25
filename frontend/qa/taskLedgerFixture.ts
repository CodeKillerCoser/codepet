// Deterministic layout data, independent of the result of a previous native UI run.
// This fixture is never used as evidence that production IPC or business actions passed.
import type {Criterion, Task, TaskRun} from '../lib/taskLedger';
export function taskLedgerFixture(): Task {
  const at=1795491000000;
  const items:Criterion[]=[0,1].map(i=>({id:`layout-check-${i}`,title:['切换窗口后点击','连续点击仍保持响应'][i],precondition:'应用运行',steps:'点击桌宠菜单',expected:'菜单正常打开',scope:'both',required:true}));
  const run:TaskRun={
    id:'layout-current',title:'修复菜单点击',kind:'bugfix',goal:'点击正常响应',solution:'修复点击事件绑定',description:'布局样例',status:'ready_for_delivery',
    validationPlan:{revision:1,items,records:[],revisions:[{revision:1,createdAt:at,summary:'布局样例',affectedAreas:[],criteria:items}]},
    agentWorkspace:{path:'/Users/qa/project',branch:'codex/layout',head:null,upstream:null,changeStatus:{hasUncommittedChanges:true,hasUnpushedCommits:false},changedFiles:[],relatedCommits:[],observedAt:at,error:null},
    agentConversation:[],timeline:[],createdBy:'codex',createdAt:at,updatedAt:at,closure:null
  };
  const previous=structuredClone(run);previous.id='layout-history';previous.status='closed';previous.createdAt-=86400000;previous.closure={accepted:false,reason:'布局样例',closedAt:at};
  return {schemaVersion:5,id:'layout-task',title:'修复桌宠菜单点击失效',description:'布局样例背景',projectId:'layout-project',projectName:'codepet',createdBy:'codex',createdAt:at-86400000,runs:[previous,run],revision:1,updatedAt:at,summary:{status:'ready_for_delivery',runCounts:{total:2,inProgress:0,readyForDelivery:1,published:0,closed:1},manuallyCompleted:false}};
}
