import { test } from 'node:test';
import assert from 'node:assert/strict';
import { appCacheItems } from '../src/lib/appCleanup.ts';
import type { CandidateItem } from '../src/types.ts';
const candidate = (path: string, category = 'rebuildable'): CandidateItem => ({ path, name: path, kind: 'directory', total: 100, category, reason: '', tag: '', action: 'quarantine', cost: 'instant' });
test('app cleanup restricts selections to known caches and removes overlapping descendants', () => {
  const home = '/Users/test';
  const root = `${home}/Library/Caches/Google/Chrome`;
  const candidates = [candidate(root), candidate(`${root}/Cache`), candidate(`${root}-other`), candidate(`${home}/Library/Application Support/Google/Chrome`), candidate(`${root}/state`, 'keep')];
  assert.deepEqual(appCacheItems(candidates, home, ['Library/Caches/Google/Chrome']).map((c) => c.path), [root]);
  assert.deepEqual(appCacheItems(candidates, '', ['Library/Caches/Google/Chrome']), []);
});
