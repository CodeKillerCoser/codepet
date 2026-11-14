use codepet_core_sdk::{RoutedResourceId, VersionRange};

#[test]
fn routed_resource_identity_round_trips_agent_route_dimensions() {
    let routed = RoutedResourceId {
        provider_id: "codex-work".to_string(),
        native_resource_id: "thread-01".to_string(),
    };

    let encoded = serde_json::to_value(&routed).unwrap();
    assert_eq!(encoded["providerId"], "codex-work");
    assert_eq!(encoded["nativeResourceId"], "thread-01");
    assert_eq!(serde_json::from_value::<RoutedResourceId>(encoded).unwrap(), routed);
}

#[test]
fn version_range_is_language_neutral_and_explicit() {
    let range = VersionRange {
        min_version: 1,
        max_version: 2,
    };

    assert_eq!(serde_json::to_string(&range).unwrap(), r#"{"minVersion":1,"maxVersion":2}"#);
}

#[test]
fn highest_common_version_is_checked_in_both_directions() {
    use codepet_core_sdk::{ProtocolVersionOffer as Offer, ProtocolVersionSelection as Selection};
    let local=Offer{version:4,min_version:2};
    for (peer, selected) in [(Offer{version:5,min_version:3},4),(Offer{version:2,min_version:2},2),(Offer{version:4,min_version:4},4)] {
        let reply=peer.negotiate(&local).unwrap();
        assert_eq!(reply.selected_version,selected); assert_eq!(reply.offer(),peer);
        local.validate_selection(&reply).unwrap();
    }
    for peer in [Offer{version:1,min_version:1},Offer{version:6,min_version:5},Offer{version:2,min_version:3},Offer{version:0,min_version:0}] { assert!(local.negotiate(&peer).is_err()); }
    assert!(local.validate_selection(&Selection{version:5,min_version:2,selected_version:2}).is_err());
}
