// Explicit opt-in cloud acceptance. Never run as part of npm test.
import { cp, mkdtemp, mkdir, writeFile, rm, readFile } from 'node:fs/promises';
import { dirname, resolve, join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { spawn, spawnSync } from 'node:child_process';
import assert from 'node:assert/strict';

export const deepseekConfig = { providers: { 'codepet-deepseek': {
  baseUrl: 'https://api.deepseek.com', api: 'openai-completions',
  // Resolved by pi at request time; no key in config, manifest, repository or logs.
  apiKey: '!/bin/zsh -ic \'print -rn -- "$DS_API_KEY" >&3\' 3>&1 1>/dev/null',
  models: [{ id: 'deepseek-flash', name: 'DeepSeek Flash', reasoning: false, input: ['text'],
    contextWindow: 1000000, maxTokens: 1024, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    compat: { supportsStore: false, supportsDeveloperRole: false, maxTokensField: 'max_tokens',
      requiresReasoningContentOnAssistantMessages: true, thinkingFormat: 'deepseek', supportsStrictMode: true } }],
} } };

if (fileURLToPath(import.meta.url) === process.argv[1]) {
  // Capture shell output privately: never forward stdout containing the key to tool output.
  const credential = spawnSync('/bin/zsh', ['-ic', 'print -rn -- "$DS_API_KEY" >&3'], { encoding: 'utf8', stdio: ['ignore', 'ignore', 'pipe', 'pipe'] });
  assert.equal(credential.status, 0, 'Could not load DS_API_KEY from zsh configuration');
  const key = credential.output[3]?.toString().trim();
  assert.ok(key && !key.includes('\n'), 'DS_API_KEY unavailable or shell stdout contains extra content');
  credential.stdout = ''; credential.stderr = '';
  const root = await mkdtemp(join(tmpdir(), 'codepet-pi-live-'));
  try {
    const agent = join(root, 'agent'); const workspace = join(root, 'workspace'); const packageDir = join(root, 'package');
    await mkdir(agent); await mkdir(workspace);
    await writeFile(join(workspace, 'proof.txt'), 'PI_VALIDATION_OK');
    await writeFile(join(agent, 'models.json'), JSON.stringify(deepseekConfig));
    const source = dirname(dirname(fileURLToPath(import.meta.url)));
    await cp(resolve(process.argv[2] ?? join(source, 'dist/pi-provider')), packageDir, { recursive: true });
    const manifestPath = join(packageDir, 'codepet-provider.json');
    const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
    manifest.env.CODEPET_PI_AGENT_DIR = agent;
    await writeFile(manifestPath, JSON.stringify(manifest));
    const child = spawn('cargo', ['test', '--manifest-path', 'crates/Cargo.toml', '-p', 'codepet-host', '--test', 'external_pi_provider', '--', '--ignored', '--nocapture'], {
      cwd: resolve(source, '../../..'), stdio: 'inherit', env: { ...process.env, CODEPET_PI_PACKAGE: manifestPath,
        CODEPET_PI_WORKSPACE: workspace, CODEPET_PI_LIVE: '1', CODEPET_PI_MODEL: 'codepet-deepseek/deepseek-flash' },
    });
    const code = await new Promise<number | null>((resolve, reject) => { child.once('error', reject); child.once('exit', resolve); });
    assert.equal(code, 0, 'Live cloud acceptance failed');
  } finally { await rm(root, { recursive: true, force: true }); }
}
