import 'dart:async';
import 'generated.dart';

/// Own one heartbeat per connected Gateway session, independently of screens.
final class GatewayHeartbeatClient {
  GatewayHeartbeatClient({
    required this.client,
    required this.onProviders,
    required this.onFailure,
    this.beforePing,
    this.interval = const Duration(milliseconds: gatewayHeartbeatIntervalMs),
    this.requestTimeout = const Duration(milliseconds: gatewayHeartbeatRequestTimeoutMs),
    this.failureTimeout = const Duration(milliseconds: gatewayHeartbeatTimeoutMs),
  });

  final ProtocolClient client;
  final void Function(List<ProviderSummary>, int? inventoryRevision, String? inventoryEpoch) onProviders;
  final void Function(Object, StackTrace) onFailure;
  final void Function()? beforePing;
  final Duration interval;
  final Duration requestTimeout;
  final Duration failureTimeout;
  final Stopwatch _sincePong = Stopwatch();
  Timer? _timer;
  int _sequence = 0;
  bool _closed = false;

  void start() {
    if (_closed || _sincePong.isRunning) return;
    _sincePong.start();
    _timer = Timer(interval, _ping);
  }

  Future<void> _ping() async {
    if (_closed) return;
    final sequence = ++_sequence;
    try {
      beforePing?.call();
      final pong = await client.protocolPing(PingRequest(sequence: sequence)).timeout(requestTimeout);
      if (_closed) return;
      if (pong.sequence != sequence) throw const FormatException('Heartbeat sequence mismatch');
      _sincePong.reset();
      onProviders(pong.providers, pong.inventoryRevision, pong.inventoryEpoch);
    } catch (error, stack) {
      if (_closed) return;
      if (_sincePong.elapsed >= failureTimeout || error is FormatException) {
        close();
        onFailure(error, stack);
        return;
      }
    }
    if (!_closed) _timer = Timer(interval, _ping);
  }

  void close() {
    _closed = true;
    _timer?.cancel();
    _sincePong.stop();
  }
}
