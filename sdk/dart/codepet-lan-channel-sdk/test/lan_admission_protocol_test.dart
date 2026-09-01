import 'dart:convert';
import 'dart:io';

import 'package:codepet_lan_channel_sdk/codepet_lan_channel_sdk.dart';
import 'package:test/test.dart';

const fixtureRoot = '../../../protocol/channel/lan/v1/fixtures';

Object? fixture(String name) =>
    jsonDecode(File('$fixtureRoot/$name').readAsStringSync());

void main() {
  test('all canonical LAN admission fixtures round-trip', () {
    final index = fixture('index.json') as List;
    for (final rawEntry in index) {
      final entry = (rawEntry as Map).cast<String, Object?>();
      final file = entry['file']! as String;
      final name = entry['name']! as String;
      final json = fixture(file);
      final encoded = switch (name) {
        'PairingQrPayload' => PairingQrPayload.fromJson(json).toJson(),
        'PairingExchangeRequest' => PairingExchangeRequest.fromJson(json).toJson(),
        'PairingExchangeResponse' => PairingExchangeResponse.fromJson(json).toJson(),
        'CurrentCredentialDeleteResponse' =>
          CurrentCredentialDeleteResponse.fromJson(json).toJson(),
        _ => throw StateError('unhandled canonical LAN fixture: $name'),
      };
      expect(encoded, equals(json), reason: file);
    }
  });

  test('secrets are constrained and redacted', () {
    final qr = PairingQrPayload.fromJson(fixture('pairing-qr-payload.json'));
    final exchange = PairingExchangeRequest.fromJson(
      fixture('pairing-exchange-request.json'),
    );
    final response = PairingExchangeResponse.fromJson(
      fixture('pairing-exchange-response.json'),
    );

    for (final pair in [
      (qr.toString(), qr.pairingSecret),
      (exchange.toString(), exchange.pairingSecret),
      (response.toString(), response.credential),
    ]) {
      expect(pair.$1, contains('<redacted>'));
      expect(pair.$1, isNot(contains(pair.$2)));
    }

    expect(
      () => PairingQrPayload.fromJson({
        ...(fixture('pairing-qr-payload.json') as Map),
        'pairingSecret': 'not-a-secret',
      }),
      throwsA(isA<ProtocolCodecException>()),
    );
  });
}
