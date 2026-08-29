use codepet_pet_sdk::{decode_event, PetSnapshotEvent, ProtocolEvent};

#[test]
fn pet_snapshot_fixture_decodes_without_provider_domain_types() {
    let event = decode_event(include_bytes!(
        "../../../../protocol/pet/v1/fixtures/snapshot-event.json"
    ))
    .unwrap();

    let ProtocolEvent::PetSnapshot { payload, .. } = event else {
        panic!("expected pet.snapshot");
    };
    let PetSnapshotEvent { snapshot } = payload;
    assert_eq!(snapshot.tasks[0].id, "pet-task-1");
}
