# CodePet Dart SDKs

The null-safe `codepet_core_sdk` and `codepet_gateway_sdk` packages are generated from the language-neutral protocol inputs under `protocol/`. Import `package:codepet_gateway_sdk/codepet_gateway_sdk.dart` from a Dart or Flutter client; it re-exports the core identifiers and errors used by Gateway DTOs.

`ProtocolClient` is transport-neutral. A consumer supplies a `ProtocolTransport` that correlates a request map with its response map and a stable request-ID factory. The generated client exposes one typed Dart method for each Gateway manifest route. Raw WebSocket framing, TLS/pairing lifecycle, credential storage, retry policy, and event persistence remain client-runtime responsibilities.

Run generation and validation from the repository root:

```sh
npm run protocol:generate
npm run protocol:check
dart pub get --directory sdk/dart/codepet-gateway-sdk
# Then run `dart test` with sdk/dart/codepet-gateway-sdk as the working directory.
```
