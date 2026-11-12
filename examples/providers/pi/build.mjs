import { cp, mkdir, writeFile, chmod, readdir } from 'node:fs/promises';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const source = dirname(fileURLToPath(import.meta.url));
const sdk = resolve(process.env.CODEPET_SDK_DIR ?? '/Applications/Code Pet.app/Contents/Resources/provider-sdk');
const output = resolve(process.argv[2] ?? join(source, 'dist/pi-provider'));
const scratch = join(source, '.build');
await mkdir(scratch, { recursive: true });
// Distribution currently lacks TS export: copy canonical generated DTOs only.
for (const name of ['codepet-core-sdk', 'codepet-agent-sdk', 'codepet-provider-sdk']) {
  await cp(resolve(source, '../../../sdk/typescript', name), join(source, 'types', name), { recursive: true });
}
execFileSync(join(sdk, 'cp-sdk-gen'), ['--package', 'provider', '--role', 'server', '--lang', 'rust', '--protocol', join(sdk, 'protocol'), '--output', join(scratch, 'exported-sdk')], { stdio: 'inherit' });
await cp(join(source, 'bridge'), join(scratch, 'bridge'), { recursive: true });
await cp(join(sdk, 'protocol/provider/v1/manifest.json'), join(scratch, 'provider-manifest.json'));
execFileSync('cargo', ['build', '--release', '--manifest-path', join(scratch, 'bridge/Cargo.toml')], { stdio: 'inherit' });
await mkdir(output, { recursive: true });
for (const name of ['provider.ts', 'server.ts', 'types', 'package.json', 'package-lock.json', 'tsconfig.json']) {
  await cp(join(source, name), join(output, name), { recursive: true });
}
await cp(join(scratch, 'bridge/target/release/codepet-provider-jsonl-bridge'), join(output, 'codepet-provider-jsonl-bridge'));
await chmod(join(output, 'codepet-provider-jsonl-bridge'), 0o755);
// The importer accepts regular files/directories only, including dependencies.
execFileSync('npm', ['ci', '--omit=dev', '--bin-links=false', '--no-audit', '--no-fund'], { cwd: output, stdio: 'inherit' });
async function checkRegular(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (entry.isSymbolicLink() || (!entry.isDirectory() && !entry.isFile())) throw new Error(`Package contains unsupported entry: ${join(directory, entry.name)}`);
    if (entry.isDirectory()) await checkRegular(join(directory, entry.name));
  }
}
await checkRegular(output);
const manifest = {
  manifestVersion: 1, transport: 'stdio-codepet-mux-v1',
  pluginId: 'dev.codepet.example.pi', displayName: 'Pi (TypeScript)',
  executable: 'codepet-provider-jsonl-bridge', args: [process.execPath, 'provider.ts'], env: {}, enabled: true,
  instances: [{ instanceId: 'pi-example', instanceKind: 'pi', displayName: 'Pi', settings: {}, enabled: true }],
};
await writeFile(join(output, 'codepet-provider.json'), JSON.stringify(manifest, null, 2) + '\n');
console.log(`Import ${join(output, 'codepet-provider.json')}`);
