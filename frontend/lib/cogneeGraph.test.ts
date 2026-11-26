import {expect, test, vi} from 'vitest';
import {taskRequest} from './taskLedger';
import {loadRecallGraph} from './cogneeGraph';
vi.mock('./taskLedger',()=>({taskRequest:vi.fn()}));
test('the initial graph does not request semantic projection; only the Semantic view does',async()=>{
  vi.mocked(taskRequest).mockResolvedValue({nodes:[],links:[]});
  await loadRecallGraph();
  expect(taskRequest).toHaveBeenLastCalledWith('recall-graph',{semantic:false});
  await loadRecallGraph(true);
  expect(taskRequest).toHaveBeenLastCalledWith('recall-graph',{semantic:true});
});
