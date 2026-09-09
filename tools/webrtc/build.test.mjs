import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { validArtifact, recipeKey, root } from './build.mjs';

test('cache rejects missing, corrupted, differently-versioned and escaping artifacts', async t => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'codepet-native-cache-'));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  await fs.mkdir(path.join(directory, 'runtime'));
  await fs.writeFile(path.join(directory, 'runtime', 'test.dll'), 'native');
  const manifest = { schema: 1, key: 'recipe', runtime: ['runtime/test.dll'], files: [{ path: 'runtime/test.dll', sha256: createHash('sha256').update('native').digest('hex') }] };
  await fs.writeFile(path.join(directory, 'artifact.json'), JSON.stringify(manifest));
  assert.ok(await validArtifact(directory, 'recipe'));
  assert.equal(await validArtifact(directory, 'new-version'), false);
  await fs.writeFile(path.join(directory, 'runtime', 'test.dll'), 'corrupt');
  assert.equal(await validArtifact(directory, 'recipe'), false);
  await fs.unlink(path.join(directory, 'runtime', 'test.dll'));
  assert.equal(await validArtifact(directory, 'recipe'), false);
  manifest.files[0].path = '../outside';
  await fs.writeFile(path.join(directory, 'artifact.json'), JSON.stringify(manifest));
  assert.equal(await validArtifact(directory, 'recipe'), false);
});
test('target and pinned recipe changes produce different cache identities', async t => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'codepet-native-recipe-'));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  for (const file of ['versions.json', 'vcpkg.json', 'build.mjs', 'smoke.c', 'triplets']) await fs.cp(path.join(root, file), path.join(directory, file), { recursive: true });
  const original = await recipeKey('x86_64-pc-windows-msvc', directory);
  assert.notEqual(original, await recipeKey('aarch64-apple-darwin', directory));
  await fs.appendFile(path.join(directory, 'versions.json'), '\n');
  assert.notEqual(original, await recipeKey('x86_64-pc-windows-msvc', directory));
});
