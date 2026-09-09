# WebRTC native SDK

Independent producer for libdatachannel **with libnice**, shared OpenSSL, and Data Channels only.
**Known blocker:** libnice 0.1.23 does not perform standard TLS in its TURN_TLS socket path.
Its implementation only adds legacy Google/MSOC pseudo-SSL framing. This SDK must not be
advertised as a TURN/TLS fallback. `versions.json` explicitly records this unavailable capability.
The application requests a target and receives a JSON descriptor; it does not manage CMake,
vcpkg, source revisions or transitive native dependencies.

```text
tools/webrtc/
  versions.json          # immutable source revisions and required configuration
  vcpkg.json             # dependency manifest; registry pinned by versions.json
  triplets/              # supported target ABI, Release runtime, macOS minimum
  build.sh / build.ps1   # convenience entry points; both call build.mjs
  build.mjs              # ensureArtifact(target) -> SDK directory
  source/                # ignored Git checkouts: libdatachannel, vcpkg
  cache/                 # ignored downloads, tools, dependency binaries, intermediates
  out/<target>/<key>/     # verified SDK; artifact.json, include/, lib/, runtime/, licenses/
```

## Request an artifact

```sh
node tools/webrtc/build.mjs --target x86_64-pc-windows-msvc
bash tools/webrtc/build.sh --target aarch64-apple-darwin
node tools/webrtc/build.mjs --check --target x86_64-apple-darwin
```

Success writes only `{ "directory": "...", "manifest": ".../artifact.json" }` to stdout;
progress and compiler logs go to stderr. `--check` validates an existing SDK without downloading
or invoking a compiler. Missing/corrupt SDKs fail in check mode; normal requests build them.

The identity covers source pins, vcpkg snapshot, recipe, target and triplets. Every exported file
has a SHA-256 digest. A matching valid SDK is returned before probing any compiler or network.
Dependency binary caches allow rebuilding the producer without rebuilding every dependency.
Both debug and release applications consume the same Release native SDK; Debug C/C++ runtimes
must not enter distributable packages. Sources, PDBs and build tools are never packaged.

Only one producer writes the shared build cache at a time. A completed SDK is published by rename
after build and C ABI smoke checks; an interrupted build cannot become a cache hit. A dead local
lock owner is recovered automatically. A live owner is waited for, never killed automatically.

## Toolchains

- Windows x64: Node.js, Git, Visual Studio C++ desktop workload, Windows SDK and CMake tools.
  `vswhere` locates the installation and the producer imports `vcvars64.bat` only into its child
  environment. No global PATH or compiler settings are changed. vcpkg obtains its pinned helper tools.
- macOS arm64/x64: Node.js, Git, Xcode command-line tools, CMake and Ninja (`brew install cmake ninja`).
  Native dependencies are built through the pinned vcpkg checkout, not taken from Homebrew libraries.
  Requesting `universal-apple-darwin` makes the producer combine both SDKs with `lipo` and cache the result.
- Cross-OS builds and Linux SDKs are not currently implemented. Unsupported targets fail explicitly.

## Application integration

`npm run tauri -- dev/build/bundle ...` uses `scripts/tauri_with_native.mjs` to request the SDK
and pass a generated Tauri config. Existing resources remain in the base config. Windows DLLs go
beside the installed EXE; macOS dylibs go into `Contents/Frameworks`, where Tauri signs them.
Dependency install paths are rewritten to `@loader_path` before publication. Licenses go into
`webrtc/licenses`. `externalBin` is deliberately unused: these are libraries, not sidecar processes.

`src-tauri/build.rs` independently requests the target SDK, including for direct Cargo builds,
and exports `CODEPET_WEBRTC_ARTIFACT` for subsequent native adapter integration. Direct Cargo
builds do not create installers; use the npm Tauri entry point to package native libraries.
Calling the bare upstream Tauri CLI bypasses this repository's packaging wrapper.

This module prepares and packages the new backend. It does **not** switch the existing Host
channel adapter from webrtc-rs to libdatachannel, and a passing ABI smoke check is not evidence
of TURN/TLS network interoperability.

See [implementation and validation](../../knowledge/40-runbooks/webrtc-native-build.md).
