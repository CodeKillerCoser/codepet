import 'dart:convert';
import 'dart:io';

import 'package:codepet_gateway_sdk/codepet_gateway_sdk.dart';
import 'package:test/test.dart';

const fixtureRoot = '../../../protocol/gateway/v1/fixtures';

Object? fixture(String name) => jsonDecode(File('$fixtureRoot/$name').readAsStringSync());

Map<String, Object?> fixtureObject(String name) =>
    (fixture(name) as Map).cast<String, Object?>();

void main() {
  test('all canonical gateway fixtures round-trip through generated codecs', () {
    final index = fixture('index.json') as List;
    for (final rawEntry in index) {
      final entry = (rawEntry as Map).cast<String, Object?>();
      final file = entry['file']! as String;
      final kind = entry['kind']! as String;
      final name = entry['name']! as String;
      final json = fixtureObject(file);

      final encoded = switch (kind) {
        'request' => ProtocolRequestEnvelope.fromJson(json).toJson(),
        'response' => ProtocolResponseEnvelope.fromJson(json).toJson(),
        'event' => ProtocolEventEnvelope.fromJson(json).toJson(),
        'type' => switch (name) {
            'PairingQrPayload' => PairingQrPayload.fromJson(json).toJson(),
            'PairingExchangeRequest' => PairingExchangeRequest.fromJson(json).toJson(),
            'PairingExchangeResponse' => PairingExchangeResponse.fromJson(json).toJson(),
            'CurrentCredentialDeleteResponse' =>
              CurrentCredentialDeleteResponse.fromJson(json).toJson(),
            'ModelCatalog' => ModelCatalog.fromJson(json).toJson(),
            _ => throw StateError('unhandled canonical type fixture: $name'),
          },
        _ => throw StateError('unhandled canonical fixture kind: $kind'),
      };

      expect(encoded, equals(json), reason: file);
    }
  });

  test('discriminated unions retain concrete Dart variants', () {
    final flat = ModelCatalog.fromJson(fixture('model-catalog-flat.json'));
    expect(flat, isA<FlatModelCatalog>());
    expect((flat as FlatModelCatalog).models.single.id, 'gpt-5');

    final grouped = ModelCatalog.fromJson(fixture('model-catalog-grouped.json'));
    expect(grouped, isA<GroupedModelCatalog>());
    expect((grouped as GroupedModelCatalog).providers.last.id, 'anthropic');

    expect(
      () => ModelSelection.fromJson({'modelId': 'gpt-5'}),
      throwsA(isA<ProtocolCodecException>()),
    );
  });

  test('required nullable values remain explicit on the wire', () {
    final response = ProtocolResponseEnvelope.fromJson(
      fixture('turn-send-response.json'),
    );
    final result = (response.response as ProtocolSuccess).result as TurnSendResponse;
    expect(result.userItem, isNull);
    expect(result.toJson(), containsPair('userItem', null));
  });

  test('closed objects and schema constraints fail closed', () {
    final handshake = fixtureObject('handshake-request.json');
    final withUnknown = Map<String, Object?>.from(handshake)..['unexpected'] = true;
    expect(
      () => ProtocolRequestEnvelope.fromJson(withUnknown),
      throwsA(isA<ProtocolCodecException>()),
    );

    expect(
      () => DeviceDescriptor.fromJson({
        'deviceName': '',
        'operatingSystem': 'Android',
        'systemVersion': '16',
      }),
      throwsA(isA<ProtocolCodecException>()),
    );
    expect(
      () => ConversationListRequest(limit: 101),
      throwsA(isA<ProtocolCodecException>()),
    );
    expect(
      () => encodeClientId(''),
      throwsA(isA<ProtocolCodecException>()),
    );
    expect(
      () => ProtocolRequestEnvelope.fromJson({
        ...handshake,
        'protocolVersion': 2,
      }),
      throwsA(isA<ProtocolCodecException>()),
    );
    expect(
      () => PairingQrPayload.fromJson({
        ...fixtureObject('pairing-qr-payload.json'),
        'pairingSecret': 'not-a-secret',
      }),
      throwsA(isA<ProtocolCodecException>()),
    );
    expect(
      () => Approval.fromJson({
        'resource': _resource('approval-1'),
        'conversation': _resource('conversation-1'),
        'turn': _resource('turn-1'),
        'kind': 'command',
        'title': 'Approve',
        'status': 'pending',
        'decisions': ['approve', 'approve'],
      }),
      throwsA(isA<ProtocolCodecException>()),
    );
  });

  test('sensitive fields are redacted from generated diagnostics', () {
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
  });

  test('method and event metadata come from the manifest IR', () {
    expect(ProtocolMethod.values, hasLength(11));
    expect(ProtocolEventName.values, hasLength(7));
    expect(ProtocolMethod.turnSend.wireName, 'turn.send');
    expect(ProtocolMethod.turnSend.idempotency, ProtocolIdempotency.nonIdempotent);
    expect(ProtocolMethod.turnSend.capability, GatewayCapability.turnSend);
    expect(ProtocolMethod.protocolHandshake.capability, isNull);
    expect(ProtocolEventName.turnOutputDelta.delivery, 'replayable');
    expect(ProtocolEventName.turnOutputDelta.scope, 'turn');
  });

  test('typed client checks correlation and decodes generated response types', () async {
    var requestId = 0;
    final client = ProtocolClient(
      _FixtureTransport(),
      requestIdFactory: () => 'dart-request-${++requestId}',
    );

    final response = await client.deviceList(DeviceListRequest());
    expect(response.devices.single.deviceId, 'device-macbook-1');
  });
}

Map<String, Object?> _resource(String nativeResourceId) => {
      'deviceId': 'device-macbook-1',
      'providerPluginId': 'dev.codepet.codex',
      'providerInstanceId': 'codex-work',
      'nativeResourceId': nativeResourceId,
    };

final class _FixtureTransport implements ProtocolTransport {
  @override
  Future<Object?> request(Map<String, Object?> request) async {
    final envelope = ProtocolRequestEnvelope.fromJson(request);
    expect(envelope.method, ProtocolMethod.deviceList);
    return ProtocolResponseEnvelope(
      id: envelope.id,
      method: envelope.method,
      response: ProtocolSuccess(
        DeviceListResponse(
          devices: [
            Device(
              deviceId: 'device-macbook-1',
              displayName: 'MacBook',
              status: DeviceStatus.online,
            ),
          ],
        ),
      ),
    ).toJson();
  }
}
