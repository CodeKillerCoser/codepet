# Protocol code generation

`protocol/codegen.json` is the language-neutral package and target manifest. The stable generator interface is `codepet.protocol.codegen/v1`: validation first builds one normalized typed IR for definitions, constraints, discriminated unions, methods, events, capabilities, and transport metadata. A target adapter receives one validated package record, the complete protocol model, that IR, and an output path, then returns deterministic source text. The Dart adapter consumes only that normalized projection for its DTO, codec, metadata, and client surface; existing Rust/TypeScript emitters retain their validated-model input until a separately tested migration. `generatorTargetRegistry` is the implementation registry for that contract.

Rust, TypeScript, and Dart have real adapters. Rust generates every SDK; TypeScript generates core and the desktop-only v0 surface. Dart generates null-safe core and Gateway v2 packages with strict codecs, discriminated unions, method/event metadata, and a transport-neutral typed client. Python remains a registered unimplemented planned adapter; selecting it fails before writing any output. Implementing a new target requires adding its adapter, changing its status from `planned`, and declaring at least one package output without introducing another handwritten model source or method table.

Provider method `dispatchLane` is also canonical manifest metadata. Rust generation projects it through `ProtocolMethod::dispatch_lane()` so the reusable stdio runtime can reserve lifecycle-control capacity without copying method names. `tools/cp-sdk-gen` reuses this compiler at runtime: it reads the distributed schema/manifest, rebuilds normalized IR, emits Core/Provider Rust source, and adds the stable package/runtime scaffold. Bun `--compile` packages the JavaScript compiler and scaffold assets into the native executable shipped beside the App's protocol resources; checked-in `generated.rs` files are not embedded or copied.

The Dart adapter deliberately emits from the normalized IR instead of delegating to quicktype. A quicktype 26.0.0 experiment proved relative cross-file `$ref` resolution works, but also showed loss of shared type names, schema constraints, closed-object rejection, sensitive-field handling, and discriminated `oneOf` exclusivity. Dart generation therefore accepts only the finite supported IR subset and rejects ambiguous or untagged unions before any output write.

Commands:

```sh
npm run protocol:generate
npm run protocol:check
node tools/protocol-codegen/generate.mjs --target=rust --check
node tools/protocol-codegen/generate.mjs --target=dart --check
```

`protocol:check` validates schemas, schema and manifest `$ref` dependency direction, capability type/method mappings, fixtures, target fail-closed behavior, normalized-IR generation, deterministic multi-package cross-schema generation, generated-file freshness, and the cross-layer boundary tests in `test.mjs`. Dart analyzer and fixture runtime tests live under `sdk/dart/codepet-gateway-sdk`.
