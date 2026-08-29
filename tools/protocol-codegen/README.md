# Protocol code generation

`protocol/codegen.json` is the language-neutral package and target manifest. The stable generator interface is `codepet.protocol.codegen/v1`: a target receives one package schema, its method/event manifest, declared dependency schemas, and an output path. A generator must resolve JSON Schema references, preserve manifest discriminators and transport metadata, and fail rather than silently approximating unsupported schema keywords.

The current implementation generates every Rust SDK plus the TypeScript core and Runtime Gateway v0 compatibility surface. Dart and Python declare the same interface as planned targets; adding either language must not introduce another handwritten model source.

Commands:

```sh
npm run protocol:generate
npm run protocol:check
```

`protocol:check` validates schemas, dependency direction, manifests, fixtures, generated-file freshness, and the cross-layer boundary tests in `test.mjs`.
