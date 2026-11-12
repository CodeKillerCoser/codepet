import { cp, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
import { fixture } from './fixture.ts';

const source = dirname(dirname(fileURLToPath(import.meta.url)));
const built = resolve(process.argv[2] ?? join(source, 'dist/pi-provider'));
const f = await fixture();
try {
  const packageDir = join(f.root, 'package'); await cp(built, packageDir, { recursive: true });
  const manifestPath = join(packageDir, 'codepet-provider.json');
  const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  manifest.env.CODEPET_PI_AGENT_DIR = f.agent;
  await writeFile(manifestPath, JSON.stringify(manifest));
  const child = spawn('cargo', ['test', '--manifest-path', 'crates/Cargo.toml', '-p', 'codepet-host', '--test', 'external_pi_provider', '--', '--ignored', '--nocapture'], {
    cwd: resolve(source, '../../..'), stdio: 'inherit', env: { ...process.env, CODEPET_PI_PACKAGE: manifestPath, CODEPET_PI_WORKSPACE: f.workspace },
  });
  const code = await new Promise<number | null>((resolve, reject) => { child.once('error', reject); child.once('exit', resolve); });
  assert.equal(code, 0, 'Native import/Host/Gateway test failed');
  assert.ok(f.counts().toolResults > 0, 'The real pi read tool must run');
  console.log('Real pi SDK used deterministic local model endpoint:', f.counts());
} finally { await f.close(); }
