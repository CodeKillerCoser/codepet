#!/usr/bin/env bun
// Use the same canonical module loader and language generator for every Remote contract.
import path from 'node:path';
import { readFile } from 'node:fs/promises';
import { runCpSdkGen } from '../tools/cp-sdk-gen/cp-sdk-gen.mjs';

export async function syncRemoteSdk(remoteRoot, { check = false, sourceRoot = path.resolve(import.meta.dirname, '..') } = {}) {
  remoteRoot = path.resolve(remoteRoot);
  const pubspec = await readFile(path.join(remoteRoot, 'pubspec.yaml'), 'utf8');
  if (!/^name:\s*codepet_remote\s*$/m.test(pubspec)) throw new Error('Target must be the CodePet Remote repository');
  const results = [];
  for (const [name, role, output] of [
    ['gateway', 'client', 'gateway'], ['lan-channel', 'models', 'lan'], ['channel-control', 'models', 'channel-control'],
  ]) results.push(await runCpSdkGen([
    '--package', name, '--role', role, '--lang', 'dart',
    '--protocol', path.join(sourceRoot, 'protocol'), '--output', path.join(remoteRoot, 'sdk', output),
    ...(check ? ['--check'] : []),
  ], sourceRoot));
  return results;
}
if (import.meta.main) {
  const args = process.argv.slice(2), root = args.find(value => value !== '--check');
  if (!root || args.some(value => value !== root && value !== '--check')) throw new Error('Usage: bun scripts/sync_remote_sdk.mjs <remote-repository> [--check]');
  for (const result of await syncRemoteSdk(root, { check: args.includes('--check') })) console.log(result);
}
