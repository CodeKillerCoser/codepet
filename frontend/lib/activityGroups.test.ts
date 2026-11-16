import { expect,it } from 'vitest';
import { groupActivities } from './activityGroups';
import { notice } from './noticeTestFixtures';
it('groups by finish and enabled input actions without interpreting custom statuses',()=>{
 const reply={id:'r',kind:'reply' as const,label:'回复',enabled:true};const waiting={...notice('wait'),actions:[reply]},done={...notice('done',2,true),actions:[reply]},run=notice('run'),disabled={...notice('disabled'),actions:[{...reply,enabled:false}]};
 expect(groupActivities([done,run,waiting,disabled]).map(g=>[g.id,g.activities.map(n=>n.noticeId)])).toEqual([['attention',['wait']],['active',['run','disabled']],['completed',['done']]]);expect(groupActivities([])).toEqual([]);
});
