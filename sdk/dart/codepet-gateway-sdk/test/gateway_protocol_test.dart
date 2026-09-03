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
        'response' => ProtocolResponseEnvelope.fromJson(
            json,
            method: ProtocolMethod.fromJson(name),
          ).toJson(),
        'event' => ProtocolEventEnvelope.fromJson(json).toJson(),
        _ => throw StateError('unhandled canonical fixture kind: $kind'),
      };

      expect(encoded, equals(json), reason: file);
    }
  });

  test('discriminated unions retain concrete Dart variants', () {
    final flat = ModelCatalog.fromJson({
      'kind': 'flat',
      'models': [
        {'id': 'gpt-5', 'displayName': 'GPT-5'},
      ],
      'defaultSelection': {'kind': 'flat', 'modelId': 'gpt-5'},
    });
    expect(flat, isA<FlatModelCatalog>());
    expect((flat as FlatModelCatalog).models.single.id, 'gpt-5');

    final grouped = ModelCatalog.fromJson({
      'kind': 'grouped',
      'providers': [
        {
          'id': 'openai',
          'displayName': 'OpenAI',
          'models': [
            {'id': 'gpt-5', 'displayName': 'GPT-5'},
          ],
        },
        {
          'id': 'anthropic',
          'displayName': 'Anthropic',
          'models': [
            {'id': 'claude-sonnet', 'displayName': 'Claude Sonnet'},
          ],
        },
      ],
      'defaultSelection': {
        'kind': 'grouped',
        'providerId': 'openai',
        'modelId': 'gpt-5',
      },
    });
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
      method: ProtocolMethod.turnSend,
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
      () => ConversationListRequest(
        projectFilter: ConversationProjectFilterAll(
          kind: ConversationProjectFilterAllKind.all,
        ),
        limit: 101,
      ),
      throwsA(isA<ProtocolCodecException>()),
    );
    expect(
      () => encodeClientId(''),
      throwsA(isA<ProtocolCodecException>()),
    );
    expect(
      () => ProtocolRequestEnvelope.fromJson({
        ...handshake,
        'jsonrpc': '1.0',
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

  test('method and event metadata come from the manifest IR', () {
    expect(ProtocolMethod.values, hasLength(18));
    expect(ProtocolEventName.values, hasLength(9));
    expect(ProtocolMethod.projectList.wireName, 'project.list');
    expect(ProtocolMethod.projectDelete.capability, GatewayCapability.projectDelete);
    expect(ProtocolEventName.projectChanged.wireName, 'project.changed');
    expect(ProtocolMethod.turnSend.wireName, 'turn.send');
    expect(ProtocolMethod.turnSend.idempotency, ProtocolIdempotency.nonIdempotent);
    expect(ProtocolMethod.turnSend.capability, GatewayCapability.turnSend);
    expect(ProtocolMethod.protocolHandshake.capability, isNull);
    expect(ProtocolEventName.turnOutputDelta.delivery, 'replayable');
    expect(ProtocolEventName.turnOutputDelta.scope, 'turn');
  });

  test('project metadata uses a typed immutable string map', () {
    final project = Project.fromJson({
      'resource': _resource('project-1'),
      'name': 'Gateway Project',
      'roots': [
        {'path': '/workspace/project'},
      ],
      'metadata': {'owner': 'gateway'},
      'position': 3,
      'createdAt': 10,
      'updatedAt': 20,
    });
    expect(project.metadata, {'owner': 'gateway'});
    expect(() => project.metadata['owner'] = 'changed', throwsUnsupportedError);
    expect(
      () => Project.fromJson({
        ...project.toJson(),
        'metadata': {'owner': 7},
      }),
      throwsA(isA<ProtocolCodecException>()),
    );
  });

  test('typed client checks correlation and decodes generated response types', () async {
    var requestId = 0;
    final client = ProtocolClient(
      _FixtureTransport(),
      requestIdFactory: () => 'dart-request-${++requestId}',
    );

    final response = await client.providerList(ProviderListRequest());
    expect(response.providers.single.id, 'codex-work');
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
    expect(envelope.method, ProtocolMethod.providerList);
    return ProtocolResponseEnvelope(
      id: envelope.id,
      method: envelope.method,
      response: ProtocolSuccess(
        ProviderListResponse(
          providers: [
            ProviderSummary(
              id: 'codex-work',
              identity: ProviderIdentity(displayName: 'Codex'),
              runtime: ProviderRuntime(status: ProviderStatus.ready),
              capabilities: ProviderCapabilitiesSummary(revision: 'codex-v1'),
            ),
          ],
        ),
      ),
    ).toJson();
  }
}
