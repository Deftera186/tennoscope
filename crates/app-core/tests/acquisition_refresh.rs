use app_core::{AppCore, HealthState};
use local_store::SnapshotMeta;
use warframe_acquisition::{
    AcquisitionError, AcquisitionFailure, AcquisitionHealth, AcquisitionResult, CatalogLoadSource,
};
use warframe_domain::{CatalogItem, Category, InventoryEntry, InventorySnapshot, ItemId};

fn snapshot(id: &str) -> InventorySnapshot {
    InventorySnapshot::coherent(vec![InventoryEntry::new(
        CatalogItem::new(ItemId::new(id).unwrap(), id, Category::Weapon).unwrap(),
        1,
    )])
    .unwrap()
}

#[test]
fn failed_refresh_preserves_the_prior_persisted_snapshot_and_records_stage_health() {
    let mut core = AppCore::in_memory().unwrap();
    core.apply_inventory_snapshot(snapshot("prior"), SnapshotMeta::fake("old").unwrap())
        .unwrap();
    let failure = AcquisitionFailure::for_test(AcquisitionError::AuthorizationNotFound);

    let view = core.finish_inventory_refresh(Err(failure), None).unwrap();

    assert_eq!(view.collection().items()[0].id(), "prior");
    assert_eq!(view.health().game_reader().state(), HealthState::Failed);
    assert!(
        view.health()
            .game_reader()
            .message()
            .contains("authorization")
    );
    assert_eq!(view.health().acquisition_stages().len(), 1);
    assert!(!view.health().game_reader().message().contains("accountId"));
}

#[test]
fn successful_refresh_atomically_replaces_snapshot_and_reports_catalog_freshness() {
    let mut core = AppCore::in_memory().unwrap();
    core.apply_inventory_snapshot(snapshot("prior"), SnapshotMeta::fake("old").unwrap())
        .unwrap();
    let result = AcquisitionResult::new(snapshot("new"), AcquisitionHealth::successful()).unwrap();
    let meta = SnapshotMeta::new(
        "2026-07-24T20:00:00Z".into(),
        "unknown".into(),
        "warframe-memory".into(),
    )
    .unwrap();

    let view = core
        .finish_inventory_refresh(
            Ok((result, meta)),
            Some((CatalogLoadSource::StaleCache, 123)),
        )
        .unwrap();

    assert_eq!(view.collection().items()[0].id(), "new");
    assert_eq!(view.health().game_reader().state(), HealthState::Ready);
    assert_eq!(view.health().catalog().state(), HealthState::Degraded);
    assert!(view.health().catalog().message().contains("cached"));
    assert_eq!(view.health().acquisition_stages().len(), 5);
}
