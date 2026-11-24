import {describe,it,expect} from 'vitest';
import {patchLines} from './taskDiffs';
describe('Git unified diff',()=>{
 it('tracks independent old/new lines across hunks and renamed sections',()=>{
  const rows=patchLines('diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -2,2 +2,3 @@\n same\n-old\n+new\n+extra\n@@ -20 +21 @@\n-z\n+q\n\\ No newline at end of file\ndiff --git a/b b/b\n--- a/b\n+++ b/b\n@@ -0,0 +1 @@\n+added\n');
  expect(rows.filter(r=>r.kind==='added').map(r=>[r.old,r.next,r.text])).toEqual([[null,3,'new'],[null,4,'extra'],[null,21,'q'],[null,1,'added']]);
  expect(rows.filter(r=>r.kind==='deleted').map(r=>r.old)).toEqual([3,20]);
  expect(rows.some(r=>r.kind==='note')).toBe(true);
 });
 it('does not interpret filenames or metadata as changed code',()=>{
  expect(patchLines('diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n')).toEqual([]);
 });
});
