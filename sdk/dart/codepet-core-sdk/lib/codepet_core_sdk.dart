/// Generated, language-neutral CodePet core protocol types and codecs.
library;

import 'src/generated.dart';
export 'src/generated.dart';

/// Highest-common-version negotiation for a continuous compatibility interval.
extension ProtocolVersionOfferNegotiation on ProtocolVersionOffer {
  ProtocolVersionSelection negotiate(ProtocolVersionOffer peer) {
    if (minVersion < 1 || minVersion > version || peer.minVersion < 1 || peer.minVersion > peer.version) {
      throw const FormatException('invalid_protocol_range');
    }
    final selected = version < peer.version ? version : peer.version;
    final minimum = minVersion > peer.minVersion ? minVersion : peer.minVersion;
    if (selected < minimum) throw const FormatException('unsupported_protocol_version');
    return ProtocolVersionSelection(version: version, minVersion: minVersion, selectedVersion: selected);
  }
  bool accepts(ProtocolVersionSelection peer) {
    try {
      return negotiate(ProtocolVersionOffer(version: peer.version, minVersion: peer.minVersion)).selectedVersion == peer.selectedVersion;
    } on FormatException { return false; }
  }
}

/// Preserved future values are receive data, never implicit executable choices.
void validateKnownProtocolChoices(Map<String, dynamic> definitions, String name, Object? value) {
  void visit(Map<String, dynamic> node, Object? value) {
    if (node['ref'] case final String key) { visit(definitions[key] as Map<String, dynamic>, value); return; }
    if (node['discriminator'] case final Map d) {
      if (value is! Map) throw const FormatException('Expected discriminated object');
      final ref = (d['variants'] as Map)[value[d['field']]];
      if (ref is! String) throw const FormatException('Unsupported request choice');
      visit(definitions[ref] as Map<String, dynamic>, value); return;
    }
    if (node['oneOf'] case final List variants) {
      if (value == null && variants.any((v) => v['type'] == 'null')) return;
      for (final variant in variants) { if (variant['type'] != 'null') visit(variant as Map<String, dynamic>, value); }
      return;
    }
    if (node['enum'] case final List values) { if (!values.contains(value)) throw const FormatException('Unsupported request enum'); }
    if (value is Map && node['properties'] is Map) {
      for (final key in (node['properties'] as Map).keys) { if (value.containsKey(key)) visit((node['properties'] as Map)[key] as Map<String, dynamic>, value[key]); }
    }
    if (value is List && node['items'] is Map<String, dynamic>) { for (final entry in value) { visit(node['items'] as Map<String, dynamic>, entry); } }
  }
  visit(definitions[name] as Map<String, dynamic>, value);
}
