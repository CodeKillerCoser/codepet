import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fuse, graphRanks } from './compare.mjs';

test('workspace and recency cannot create relevance or exclude a cross-workspace hit',()=>{
  const documents=[{runId:'recent-local',workspace:'/a',updatedAt:99},
    {runId:'relevant-remote',workspace:'/b',updatedAt:1}];
  assert.deepEqual(fuse({semantic:['relevant-remote'],lexical:[],graph:[]},documents,'/a'),['relevant-remote']);
  assert.deepEqual(fuse({semantic:[],lexical:[],graph:[]},documents,'/a'),[]);
});
test('graph provenance repeated across chunks does not multiply an entity vote',()=>{
  const rows=[['a','x'],['a','x'],['a','x'],['b','x'],['b','y']];
  assert.deepEqual(graphRanks(rows),['b','a']);
});
test('fusion keeps distinct Runs belonging to the same topic',()=>{
  const docs=[{runId:'r1',taskId:'topic',workspace:'/a',updatedAt:1},
    {runId:'r2',taskId:'topic',workspace:'/b',updatedAt:2}];
  const result=fuse({semantic:['r1','r2'],lexical:['r2'],graph:[]},docs,'/a');
  assert.equal(result.length,2);
  assert(result.includes('r1')&&result.includes('r2'));
});
