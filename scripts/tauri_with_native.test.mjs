import test from 'node:test';
import assert from 'node:assert/strict';
import { bundleConfig, requestedTarget } from './tauri_with_native.mjs';

test('explicit target wins over environment and supports universal builds', () => {
  assert.equal(requestedTarget(['build', '--target', 'universal-apple-darwin'], { CODEPET_WEBRTC_TARGET: 'wrong' }), 'universal-apple-darwin');
  assert.equal(requestedTarget(['build', '--target=x86_64-pc-windows-msvc'], {}), 'x86_64-pc-windows-msvc');
});
test('package only runtime and licenses; DLLs are beside EXE, dylibs are signed frameworks', () => {
  const artifacts = [{ sdk: '/sdk', manifest: { runtime: ['runtime/datachannel.dll'], files: [{ path: 'licenses/libnice.txt' }, { path: 'include/rtc.h' }] } }];
  const windows = bundleConfig('x86_64-pc-windows-msvc', artifacts, '/stage');
  assert.deepEqual(Object.values(windows.bundle.resources).sort(), ['datachannel.dll', 'webrtc/licenses/libnice.txt']);
  artifacts[0].manifest.runtime = ['runtime/libdatachannel.dylib'];
  const mac = bundleConfig('aarch64-apple-darwin', artifacts, '/stage');
  assert.equal(mac.bundle.macOS.frameworks.length, 1);
  assert.ok(mac.bundle.macOS.frameworks[0].endsWith('libdatachannel.dylib'));
  assert.deepEqual(Object.values(mac.bundle.resources), ['webrtc/licenses/libnice.txt']);
});
