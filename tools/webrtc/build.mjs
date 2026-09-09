#!/usr/bin/env node
// Standalone native SDK producer. Only the manifest is consumed by the application.
import fs from 'node:fs/promises';
import { existsSync } from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';

export const root = path.dirname(fileURLToPath(import.meta.url));
const sha = value => createHash('sha256').update(value).digest('hex');
const log = message => process.stderr.write(`[webrtc] ${message}\n`);
const targets = {
  'x86_64-pc-windows-msvc': { platform: 'win32', triplet: 'codepet-x64-windows', arch: 'x64' },
  'aarch64-apple-darwin': { platform: 'darwin', triplet: 'codepet-arm64-osx', arch: 'arm64' },
  'x86_64-apple-darwin': { platform: 'darwin', triplet: 'codepet-x64-osx', arch: 'x86_64' },
};
export function defaultTarget() {
  if (process.platform === 'win32' && process.arch === 'x64') return 'x86_64-pc-windows-msvc';
  if (process.platform === 'darwin') return process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin';
  throw new Error('Native WebRTC SDK currently supports Windows x64 and macOS arm64/x64.');
}
function run(command, args, { env = process.env, cwd = root, capture = false } = {}) {
  const result = spawnSync(command, args, { cwd, env, encoding: 'utf8', windowsHide: true, windowsVerbatimArguments: command === 'cmd.exe',
    stdio: capture ? ['ignore', 'pipe', 'pipe'] : ['ignore', 2, 2], maxBuffer: 16 * 1024 * 1024 });
  if (result.error || result.status !== 0) throw new Error(`${command} failed: ${result.error?.message ?? result.stderr ?? result.status}`);
  return result.stdout?.trim() ?? '';
}
async function files(directory, prefix = '') {
  const result = [];
  for (const item of await fs.readdir(directory, { withFileTypes: true })) {
    const relative = path.join(prefix, item.name);
    if (item.isDirectory()) result.push(...await files(path.join(directory, item.name), relative));
    else result.push(relative);
  }
  return result.sort();
}
export async function recipeKey(target, directory = root) {
  const inputs = ['versions.json', 'vcpkg.json', 'build.mjs', 'smoke.c', ...(await files(path.join(directory, 'triplets'))).map(f => `triplets/${f}`)];
  return sha(JSON.stringify([target, ...(await Promise.all(inputs.map(async f => [f, sha(await fs.readFile(path.join(directory, f)))])))]));
}
export async function validArtifact(directory, key) {
  try {
    const manifest = JSON.parse(await fs.readFile(path.join(directory, 'artifact.json'), 'utf8'));
    if (manifest.schema !== 1 || manifest.key !== key || !manifest.files?.length || !manifest.runtime?.length) return false;
    if (!manifest.runtime.every(file => manifest.files.some(entry => entry.path === file))) return false;
    for (const file of manifest.files) {
      const resolved = path.resolve(directory, file.path);
      if (!resolved.startsWith(path.resolve(directory) + path.sep)) return false;
      if (sha(await fs.readFile(resolved)) !== file.sha256) return false;
    }
    return manifest;
  } catch { return false; }
}
async function removeOwned(directory) {
  const resolved = path.resolve(directory);
  if (!['cache', 'out', '.build-lock'].some(name => resolved === path.join(root, name) || resolved.startsWith(path.join(root, name) + path.sep))) {
    throw new Error(`Refusing to remove a directory outside generated WebRTC storage: ${resolved}`);
  }
  await fs.rm(resolved, { recursive: true, force: true });
}
async function withLock(callback) {
  const lock = path.join(root, '.build-lock');
  const deadline = Date.now() + 60 * 60 * 1000;
  let announced = false;
  while (true) {
    try { await fs.mkdir(lock); await fs.writeFile(path.join(lock, 'owner.json'), JSON.stringify({ pid: process.pid, host: os.hostname() })); break; }
    catch (error) {
      if (error.code !== 'EEXIST') throw error;
      try {
        const owner = JSON.parse(await fs.readFile(path.join(lock, 'owner.json'), 'utf8'));
        if (owner.host === os.hostname()) {
          try { process.kill(owner.pid, 0); }
          catch (e) { if (e.code === 'ESRCH') { await removeOwned(lock); continue; } }
        }
      } catch { /* An owner may still be writing its PID; never break a live lock by age. */ }
      if (Date.now() > deadline) throw new Error('Timed out waiting for WebRTC build lock. Check the owner before removing tools/webrtc/.build-lock.');
      if (!announced) { log('Waiting for another native SDK build'); announced = true; }
      await new Promise(resolve => setTimeout(resolve, 1000));
    }
  }
  try { return await callback(); } finally { await removeOwned(lock); }
}
async function checkout(name, spec) {
  const directory = path.join(root, 'source', name);
  if (!existsSync(path.join(directory, '.git'))) {
    await fs.mkdir(directory, { recursive: true });
    run('git', ['init', directory]);
    run('git', ['remote', 'add', 'origin', spec.repository], { cwd: directory });
  }
  if (run('git', ['status', '--porcelain', '--untracked-files=no'], { cwd: directory, capture: true })) throw new Error(`Source has local edits: ${directory}`);
  const head = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: directory, encoding: 'utf8', windowsHide: true }).stdout?.trim();
  if (head !== spec.revision) {
    run('git', ['fetch', '--depth=1', 'origin', spec.revision], { cwd: directory });
    run('git', ['checkout', '--detach', spec.revision], { cwd: directory });
  }
  if (name === 'libdatachannel') run('git', ['submodule', 'update', '--init', '--recursive', '--depth=1'], { cwd: directory });
  return directory;
}
function windowsEnvironment() {
  const vswhere = path.join(process.env['ProgramFiles(x86)'] || 'C:/Program Files (x86)', 'Microsoft Visual Studio/Installer/vswhere.exe');
  const vs = run(vswhere, ['-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath'], { capture: true });
  if (!vs) throw new Error('Install Visual Studio C++ desktop tools and Windows SDK.');
  const setup = path.join(vs, 'VC/Auxiliary/Build/vcvars64.bat');
  // No user-supplied shell fragments: this path comes from vswhere.
  const output = run('cmd.exe', ['/d', '/s', '/c', `""${setup}" >nul && set"`], { capture: true });
  const env = { ...process.env };
  for (const line of output.split(/\r?\n/)) {
    const index = line.indexOf('=');
    if (index <= 0) continue;
    const name = line.slice(0, index);
    for (const existing of Object.keys(env)) if (existing.toLowerCase() === name.toLowerCase()) delete env[existing];
    env[name] = line.slice(index + 1);
  }
  const cmake = path.join(vs, 'Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe');
  const ninja = path.join(vs, 'Common7/IDE/CommonExtensions/Microsoft/CMake/Ninja/ninja.exe');
  if (!existsSync(cmake) || !existsSync(ninja)) throw new Error('Install the Visual Studio C++ CMake tools component.');
  return { env, cmake, ninja, compiler: `${vs}; MSVC ${env.VCToolsVersion}` };
}
function toolchain(target) {
  if (targets[target].platform !== process.platform) throw new Error(`Build ${target} on ${targets[target].platform}; cross-OS builds are not supported.`);
  if (process.platform === 'win32') return windowsEnvironment();
  run('xcrun', ['--find', 'clang']);
  run('cmake', ['--version']); run('ninja', ['--version']);
  return { env: { ...process.env }, cmake: 'cmake', ninja: 'ninja', compiler: run('xcrun', ['clang', '--version'], { capture: true }) };
}
async function produce(target, key, tc) {
  const versions = JSON.parse(await fs.readFile(path.join(root, 'versions.json')));
  const source = await checkout('libdatachannel', versions.libdatachannel);
  const vcpkg = await checkout('vcpkg', versions.vcpkg);
  const executable = path.join(vcpkg, process.platform === 'win32' ? 'vcpkg.exe' : 'vcpkg');
  const bootstrapMarker = path.join(vcpkg, '.codepet-bootstrap');
  if (!existsSync(executable) || !existsSync(bootstrapMarker) || (await fs.readFile(bootstrapMarker, 'utf8')) !== versions.vcpkg.revision) {
    if (process.platform === 'win32') run('cmd.exe', ['/d', '/s', '/c', `""${path.join(vcpkg, 'bootstrap-vcpkg.bat')}" -disableMetrics"`], tc);
    else run('sh', [path.join(vcpkg, 'bootstrap-vcpkg.sh'), '-disableMetrics'], tc);
    await fs.writeFile(bootstrapMarker, versions.vcpkg.revision);
  }
  const work = path.join(root, 'cache', target, key);
  const installed = path.join(work, 'vcpkg_installed');
  const triplet = targets[target].triplet;
  const env = { ...tc.env, VCPKG_ROOT: vcpkg, VCPKG_DOWNLOADS: path.join(root, 'cache/downloads'), VCPKG_DEFAULT_BINARY_CACHE: path.join(root, 'cache/binaries'), VCPKG_DISABLE_METRICS: '1' };
  for (const directory of [work, env.VCPKG_DOWNLOADS, env.VCPKG_DEFAULT_BINARY_CACHE]) await fs.mkdir(directory, { recursive: true });
  const hostTriplet = targets[defaultTarget()].triplet;
  run(executable, ['install', `--triplet=${triplet}`, `--host-triplet=${hostTriplet}`, `--overlay-triplets=${path.join(root, 'triplets')}`, `--x-install-root=${installed}`, `--x-buildtrees-root=${path.join(root, 'cache/buildtrees')}`, `--x-packages-root=${path.join(root, 'cache/packages')}`], { env });
  const dependency = path.join(installed, triplet);
  const pkgconf = (await files(installed)).find(f => /tools[/\\]pkgconf[/\\]pkgconf(?:\.exe)?$/.test(f));
  if (!pkgconf) throw new Error('vcpkg did not produce pkgconf');
  env.PKG_CONFIG_PATH = [path.join(dependency, 'lib/pkgconfig'), path.join(dependency, 'share/pkgconfig')].join(path.delimiter);
  env.PKG_CONFIG_LIBDIR = env.PKG_CONFIG_PATH;
  const stage = path.join(work, 'sdk');
  await removeOwned(stage);
  await fs.mkdir(stage, { recursive: true });
  const options = ['-S', source, '-B', path.join(work, 'cmake'), '-G', 'Ninja',
    `-DCMAKE_MAKE_PROGRAM=${tc.ninja}`, '-DCMAKE_BUILD_TYPE=Release', '-DCMAKE_POLICY_VERSION_MINIMUM=3.5', `-DCMAKE_INSTALL_PREFIX=${stage}`,
    `-DCMAKE_TOOLCHAIN_FILE=${path.join(vcpkg, 'scripts/buildsystems/vcpkg.cmake')}`, '-DVCPKG_MANIFEST_MODE=OFF',
    `-DVCPKG_INSTALLED_DIR=${installed}`, `-DVCPKG_TARGET_TRIPLET=${triplet}`, `-DVCPKG_OVERLAY_TRIPLETS=${path.join(root, 'triplets')}`,
    `-DPKG_CONFIG_EXECUTABLE=${path.join(installed, pkgconf)}`, '-DUSE_NICE=ON', '-DNO_MEDIA=ON', '-DNO_WEBSOCKET=ON', '-DNO_TESTS=ON', '-DNO_EXAMPLES=ON', '-DBUILD_SHARED_LIBS=ON'];
  if (process.platform === 'darwin') options.push(`-DCMAKE_OSX_ARCHITECTURES=${targets[target].arch}`, `-DCMAKE_OSX_DEPLOYMENT_TARGET=${versions.macosDeploymentTarget}`);
  run(tc.cmake, options, { env });
  run(tc.cmake, ['--build', path.join(work, 'cmake'), '--parallel', String(Math.min(os.availableParallelism(), 8))], { env });
  run(tc.cmake, ['--install', path.join(work, 'cmake')], { env });
  const runtimeDir = path.join(stage, 'runtime');
  await fs.mkdir(runtimeDir);
  const aliases = new Map();
  const copied = new Set();
  for (const directory of [stage, dependency]) for (const file of await files(directory)) {
    if ((process.platform === 'win32' ? /^bin[/\\].+\.dll$/i : /^lib[/\\][^/\\]+\.dylib$/).test(file)) {
      const realFile = await fs.realpath(path.join(directory, file));
      const name = process.platform === 'darwin' && path.basename(file).startsWith('libdatachannel') ? 'libdatachannel.dylib' : path.basename(realFile);
      aliases.set(path.basename(file), name);
      if (!copied.has(name)) await fs.copyFile(realFile, path.join(runtimeDir, name));
      copied.add(name);
    }
  }
  if (process.platform === 'win32') {
    const redist = env.VCToolsRedistDir;
    if (!redist) throw new Error('Visual Studio did not expose its C++ redistributable directory');
    const x64 = path.join(redist, 'x64');
    const crt = (await fs.readdir(x64)).find(name => /^Microsoft\.VC\d+\.CRT$/.test(name));
    if (!crt) throw new Error('Missing x64 C++ redistributable runtime');
    for (const name of await fs.readdir(path.join(x64, crt))) if (name.endsWith('.dll')) {
      await fs.copyFile(path.join(x64, crt, name), path.join(runtimeDir, name));
    }
  }
  if (process.platform === 'darwin') {
    const dylibs = await fs.readdir(runtimeDir);
    for (const file of dylibs) {
      const library = path.join(runtimeDir, file);
      run('install_name_tool', ['-id', `@rpath/${file}`, library]);
      for (const line of run('otool', ['-L', library], { capture: true }).split('\n').slice(1)) {
        const dependencyPath = line.trim().split(' (')[0];
        const bundled = aliases.get(path.basename(dependencyPath));
        if (bundled) run('install_name_tool', ['-change', dependencyPath, `@loader_path/${bundled}`, library]);
        else if (!dependencyPath.startsWith('/usr/lib/') && !dependencyPath.startsWith('/System/Library/')) throw new Error(`Unbundled dylib: ${dependencyPath}`);
      }
      run('codesign', ['--force', '--sign', '-', library]);
    }
  }
  const licenses = path.join(stage, 'licenses');
  await fs.mkdir(licenses);
  await fs.copyFile(path.join(source, 'LICENSE'), path.join(licenses, 'libdatachannel.txt'));
  if (process.platform === 'win32') await fs.writeFile(path.join(licenses, 'microsoft-runtime.txt'),
    `Microsoft Visual C++ runtime, redistributed from the installed Visual Studio Redist directory.\n${tc.compiler}\nhttps://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files\n`);
  for (const file of await files(path.join(installed, triplet, 'share'))) if (path.basename(file) === 'copyright') {
    await fs.copyFile(path.join(installed, triplet, 'share', file), path.join(licenses, `${path.dirname(file).replaceAll(path.sep, '-')}.txt`));
  }
  // Includes the statically embedded usrsctp and plog dependencies.
  for (const [name, filename] of [['usrsctp', 'LICENSE.md'], ['plog', 'LICENSE']]) {
    await fs.copyFile(path.join(source, 'deps', name, filename), path.join(licenses, `${name}.txt`));
  }
  const runtime = (await fs.readdir(runtimeDir)).map(name => `runtime/${name}`);
  if (!runtime.some(f => /datachannel/.test(f)) || !runtime.some(f => /nice/.test(f))) throw new Error('Missing libdatachannel/libnice runtime');
  // Compile and load the C ABI with only the runtime directory available.
  if (process.platform === 'win32') {
    run('cl.exe', ['/nologo', '/MD', `/I${path.join(stage, 'include')}`, path.join(root, 'smoke.c'), path.join(stage, 'lib/datachannel.lib'), `/Fe:${path.join(work, 'smoke.exe')}`, `/Fo:${path.join(work, 'smoke.obj')}`], { env });
    const pathKey = Object.keys(env).find(k => k.toLowerCase() === 'path') || 'PATH';
    run(path.join(work, 'smoke.exe'), [], { env: { ...env, [pathKey]: `${runtimeDir}${path.delimiter}${env[pathKey]}` } });
  } else {
    run('xcrun', ['clang', '-arch', targets[target].arch, `-I${path.join(stage, 'include')}`, path.join(root, 'smoke.c'), `-L${runtimeDir}`, '-ldatachannel', `-Wl,-rpath,${runtimeDir}`, '-o', path.join(work, 'smoke')]);
    if ((process.arch === 'arm64' ? 'arm64' : 'x86_64') === targets[target].arch) run(path.join(work, 'smoke'), []);
  }
  const manifest = { schema: 1, key, target, versions, compiler: tc.compiler, runtime,
    include: 'include', lib: 'lib', files: await Promise.all((await files(stage)).map(async file => ({ path: file.replaceAll(path.sep, '/'), sha256: sha(await fs.readFile(path.join(stage, file))) }))) };
  await fs.writeFile(path.join(stage, 'artifact.json'), JSON.stringify(manifest, null, 2) + '\n');
  const destination = path.join(root, 'out', target, key);
  await removeOwned(destination);
  await fs.mkdir(path.dirname(destination), { recursive: true });
  await fs.rename(stage, destination);
  return destination;
}
export async function ensureArtifact(target = defaultTarget(), { check = false } = {}) {
  if (!targets[target] && target !== 'universal-apple-darwin') throw new Error(`Unsupported WebRTC target: ${target}`);
  const key = await recipeKey(target);
  const destination = path.join(root, 'out', target, key);
  if (await validArtifact(destination, key)) { log(`Using verified cached SDK: ${target}`); return destination; }
  if (check) throw new Error(`No valid cached SDK for ${target}. Run tools/webrtc/build.sh --target ${target}`);
  if (target === 'universal-apple-darwin') {
    if (process.platform !== 'darwin') throw new Error('Build universal macOS SDKs on macOS');
    const slices = [];
    for (const architecture of ['aarch64-apple-darwin', 'x86_64-apple-darwin']) slices.push(await ensureArtifact(architecture));
    return withLock(async () => {
      if (await validArtifact(destination, key)) return destination;
      const manifests = await Promise.all(slices.map(async sdk => JSON.parse(await fs.readFile(path.join(sdk, 'artifact.json')))));
      if (JSON.stringify([...manifests[0].runtime].sort()) !== JSON.stringify([...manifests[1].runtime].sort())) throw new Error('Universal SDK runtime names differ between architectures');
      const stage = path.join(root, 'cache', target, key, 'sdk');
      await removeOwned(stage);
      await fs.mkdir(path.join(stage, 'runtime'), { recursive: true });
      await fs.cp(path.join(slices[0], 'include'), path.join(stage, 'include'), { recursive: true });
      await fs.cp(path.join(slices[0], 'licenses'), path.join(stage, 'licenses'), { recursive: true });
      for (const file of manifests[0].runtime) {
        const library = path.join(stage, file);
        run('lipo', ['-create', ...slices.map(sdk => path.join(sdk, file)), '-output', library]);
        run('lipo', ['-verify_arch', 'arm64', 'x86_64', library]);
        run('codesign', ['--force', '--sign', '-', library]);
      }
      await fs.mkdir(path.join(stage, 'lib'));
      await fs.copyFile(path.join(stage, 'runtime/libdatachannel.dylib'), path.join(stage, 'lib/libdatachannel.dylib'));
      const manifest = { ...manifests[0], target, key, slices: manifests.map(m => ({ target: m.target, key: m.key, compiler: m.compiler })),
        files: await Promise.all((await files(stage)).map(async file => ({ path: file.replaceAll(path.sep, '/'), sha256: sha(await fs.readFile(path.join(stage, file))) }))) };
      await fs.writeFile(path.join(stage, 'artifact.json'), JSON.stringify(manifest, null, 2) + '\n');
      await removeOwned(destination);
      await fs.mkdir(path.dirname(destination), { recursive: true });
      await fs.rename(stage, destination);
      return destination;
    });
  }
  return withLock(async () => {
    if (await validArtifact(destination, key)) return destination;
    log(`Building native SDK for ${target}`);
    return produce(target, key, toolchain(target));
  });
}
async function main() {
  let target = process.env.CODEPET_WEBRTC_TARGET || process.env.TAURI_ENV_TARGET_TRIPLE;
  let check = false;
  for (let i = 2; i < process.argv.length; i++) {
    if (process.argv[i] === '--target') target = process.argv[++i];
    else if (process.argv[i] === '--check') check = true;
    else throw new Error(`Unknown argument ${process.argv[i]}`);
  }
  const directory = await ensureArtifact(target || defaultTarget(), { check });
  process.stdout.write(JSON.stringify({ directory, manifest: path.join(directory, 'artifact.json') }) + '\n');
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main().catch(error => { log(error.stack); process.exitCode = 1; });
