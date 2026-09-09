#!/usr/bin/env node
// Application composition only: ask for an SDK, pass its runtime files to Tauri.
import { ensureArtifact, defaultTarget } from '../tools/webrtc/build.mjs';
import fs from 'node:fs/promises';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export function requestedTarget(args, env = process.env) {
  const index = args.indexOf('--target');
  return (index >= 0 ? args[index + 1] : args.find(a => a.startsWith('--target='))?.slice(9)) || env.CODEPET_WEBRTC_TARGET || defaultTarget();
}
export function bundleConfig(target, artifacts) {
  const resources = {};
  const frameworks = [];
  for (const { sdk, manifest } of artifacts) {
    for (const file of manifest.files.filter(f => f.path.startsWith('licenses/'))) {
      resources[path.join(sdk, file.path)] = `webrtc/licenses/${path.basename(file.path)}`;
    }
    for (const file of manifest.runtime) {
      if (target.includes('windows')) resources[path.join(sdk, file)] = path.basename(file);
      else frameworks.push(path.join(sdk, file));
    }
  }
  return { bundle: { resources, ...(frameworks.length ? { macOS: { frameworks: [...new Set(frameworks)] } } : {}) } };
}
function run(command, args) {
  const result = spawnSync(command, args, { cwd: repository, stdio: 'inherit', windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited with ${result.status}`);
}
async function main() {
  const args = process.argv.slice(2);
  if (['build', 'dev', 'bundle'].includes(args[0]) && !args.some(a => ['--help', '-h'].includes(a))) {
    const target = requestedTarget(args);
    const sdk = await ensureArtifact(target);
    const artifacts = [{ sdk, manifest: JSON.parse(await fs.readFile(path.join(sdk, 'artifact.json'))) }];
    const directory = path.join(repository, 'src-tauri', 'native', 'webrtc', target);
    await fs.mkdir(directory, { recursive: true });
    const configPath = path.join(directory, 'tauri-native.json');
    // Tauri merges resource maps with the existing sound/provider resource map.
    await fs.writeFile(configPath, JSON.stringify(bundleConfig(target, artifacts), null, 2));
    args.push('--config', configPath);
  }
  run(process.execPath, [path.join(repository, 'node_modules/@tauri-apps/cli/tauri.js'), ...args]);
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().catch(error => { console.error(error); process.exitCode = 1; });
