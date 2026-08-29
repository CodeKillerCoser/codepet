use codepet_core_sdk::{RoutedResourceId, VersionRange};

#[test]
fn routed_resource_identity_round_trips_all_route_dimensions() {
    let routed = RoutedResourceId {
        device_id: "device-macbook-1".to_string(),
        provider_instance_id: "codex-work".to_string(),
        native_resource_id: "thread-01".to_string(),
    };

    let encoded = serde_json::to_value(&routed).unwrap();
    assert_eq!(encoded["deviceId"], "device-macbook-1");
    assert_eq!(encoded["providerInstanceId"], "codex-work");
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
