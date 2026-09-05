import 'dart:async';
import 'package:codepet_gateway_sdk/codepet_gateway_sdk.dart';
import 'package:test/test.dart';

final class Transport implements ProtocolTransport {
  final requests = <Map<String, Object?>>[];
  final replies = <Completer<Object?>>[];
  @override
  Future<Object?> request(Map<String, Object?> request) {
    requests.add(request);
    final reply = Completer<Object?>();
    replies.add(reply);
    return reply.future;
  }
  void pong(int index, {int? sequence}) {
    final request = requests[index];
    replies[index].complete({'jsonrpc': '2.0', 'id': request['id'], 'result': {
      'sequence': sequence ?? (request['params'] as Map)['sequence'], 'providers': [],
    }});
  }
}

void main() {
  test('one request at a time; close discards a late pong', () async {
    final transport = Transport();
    var snapshots = 0;
    var id = 0;
    final heartbeat = GatewayHeartbeatClient(
      client: ProtocolClient(transport, requestIdFactory: () => '${id++}'),
      interval: const Duration(milliseconds: 5),
      onProviders: (_) => snapshots++, onFailure: (e, s) => fail('$e'),
    )..start();
    await Future<void>.delayed(const Duration(milliseconds: 25));
    expect(transport.requests.length, 1);
    heartbeat.close();
    transport.pong(0);
    await Future<void>.delayed(const Duration(milliseconds: 15));
    expect(snapshots, 0);
    expect(transport.requests.length, 1);
  });

  test('wrong sequence fails the connection, not just the current request', () async {
    final transport = Transport();
    final failed = Completer<Object>();
    final heartbeat = GatewayHeartbeatClient(
      client: ProtocolClient(transport, requestIdFactory: () => 'ping'),
      interval: const Duration(milliseconds: 5),
      onProviders: (_) => fail('invalid pong was accepted'),
      onFailure: (e, s) => failed.complete(e),
    )..start();
    await Future<void>.delayed(const Duration(milliseconds: 15));
    transport.pong(0, sequence: 999);
    expect(await failed.future.timeout(const Duration(seconds: 1)), isA<FormatException>());
    heartbeat.close();
  });
}
