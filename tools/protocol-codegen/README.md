# Protocol code generation

`protocol/codegen.json` is the language-neutral package and target manifest. The stable generator interface is `codepet.protocol.codegen/v1`: a target adapter receives one validated package record, the complete protocol model (including declared dependency schemas), and an output path, then returns deterministic source text. `generatorTargetRegistry` is the implementation registry for that contract.

Rust and TypeScript have real adapters. Rust generates every SDK; TypeScript generates core plus the Runtime Gateway v0 compatibility surface. Dart and Python are registered as unimplemented planned adapters. Selecting either one fails before writing any output; it never falls through to TypeScript. Implementing a new target requires adding its adapter, changing its status from `planned`, and declaring at least one package output without introducing another handwritten model source.

Commands:

```sh
npm run protocol:generate
npm run protocol:check
node tools/protocol-codegen/generate.mjs --target=rust --check
```

`protocol:check` validates schemas, schema and manifest `$ref` dependency direction, capability type/method mappings, fixtures, target fail-closed behavior, multi-package cross-schema generation, generated-file freshness, and the cross-layer boundary tests in `test.mjs`.
