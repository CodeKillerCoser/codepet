use codepet_lan_channel_sdk::{
    CurrentCredentialDeleteResponse, PairingExchangeRequest, PairingExchangeResponse,
    PairingQrPayload,
};

#[test]
fn lan_admission_fixtures_use_generated_models() {
    let qr: PairingQrPayload = serde_json::from_slice(include_bytes!(
        "../../../../protocol/channel/lan/v1/fixtures/pairing-qr-payload.json"
    ))
    .unwrap();
    let request: PairingExchangeRequest = serde_json::from_slice(include_bytes!(
        "../../../../protocol/channel/lan/v1/fixtures/pairing-exchange-request.json"
    ))
    .unwrap();
    let response: PairingExchangeResponse = serde_json::from_slice(include_bytes!(
        "../../../../protocol/channel/lan/v1/fixtures/pairing-exchange-response.json"
    ))
    .unwrap();
    let deleted: CurrentCredentialDeleteResponse = serde_json::from_slice(include_bytes!(
        "../../../../protocol/channel/lan/v1/fixtures/credential-delete-response.json"
    ))
    .unwrap();

    assert_eq!(qr.version, 1);
    assert_eq!(request.client_id, "remote-client-phone-1");
    assert_eq!(response.device.device_id, qr.host_device_id);
    assert!(response.gateway_url.ends_with("/remote/v1/gateway"));
    assert!(deleted.revoked);
}

#[test]
fn lan_secrets_are_redacted_from_debug_output() {
    let qr: PairingQrPayload = serde_json::from_slice(include_bytes!(
        "../../../../protocol/channel/lan/v1/fixtures/pairing-qr-payload.json"
    ))
    .unwrap();
    let request: PairingExchangeRequest = serde_json::from_slice(include_bytes!(
        "../../../../protocol/channel/lan/v1/fixtures/pairing-exchange-request.json"
    ))
    .unwrap();
    let response: PairingExchangeResponse = serde_json::from_slice(include_bytes!(
        "../../../../protocol/channel/lan/v1/fixtures/pairing-exchange-response.json"
    ))
    .unwrap();

    assert!(!format!("{qr:?}").contains(&qr.pairing_secret));
    assert!(!format!("{request:?}").contains(&request.pairing_secret));
    assert!(!format!("{response:?}").contains(&response.credential));
}
