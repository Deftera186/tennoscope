use app_core::{AcquisitionPort, AppCore, HealthState, InventoryRefreshOutcome};
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

struct FakePort(InventoryRefreshOutcome);
impl AcquisitionPort for FakePort {
    fn refresh(&self) -> InventoryRefreshOutcome {
        self.0.clone()
    }
}

#[test]
fn acquisition_port_is_the_single_refresh_seam_and_failure_keeps_last_success() {
    let mut core = AppCore::in_memory().unwrap();
    let meta = SnapshotMeta::new("123".into(), "build".into(), "warframe-memory".into()).unwrap();
    let result =
        AcquisitionResult::new(snapshot("prior"), AcquisitionHealth::successful()).unwrap();
    core.refresh_from(&FakePort(InventoryRefreshOutcome::success(
        result,
        meta,
        CatalogLoadSource::Network,
        100,
    )))
    .unwrap();

    let failed = FakePort(InventoryRefreshOutcome::acquisition_failed(
        AcquisitionFailure::from_error(AcquisitionError::GameNotRunning),
    ));
    let view = core.refresh_from(&failed).unwrap();

    assert_eq!(view.collection().items()[0].id(), "prior");
    assert_eq!(view.health().game_reader().state(), HealthState::Degraded);
    assert_eq!(view.health().game_reader().last_success(), Some("123"));
}

#[test]
fn catalog_port_failure_is_published_without_replacing_collection() {
    let mut core = AppCore::in_memory().unwrap();
    core.apply_inventory_snapshot(snapshot("prior"), SnapshotMeta::fake("old").unwrap())
        .unwrap();
    let view = core
        .refresh_from(&FakePort(InventoryRefreshOutcome::catalog_failed()))
        .unwrap();
    assert_eq!(view.collection().items()[0].id(), "prior");
    assert_eq!(view.health().catalog().state(), HealthState::Failed);
}

#[test]
fn log_monitor_failure_never_overwrites_successful_acquisition_health() {
    let mut core = AppCore::in_memory().unwrap();
    let meta = SnapshotMeta::new("123".into(), "build".into(), "warframe-memory".into()).unwrap();
    let result =
        AcquisitionResult::new(snapshot("owned"), AcquisitionHealth::successful()).unwrap();
    core.refresh_from(&FakePort(InventoryRefreshOutcome::success(
        result,
        meta,
        CatalogLoadSource::Network,
        100,
    )))
    .unwrap();

    let view = core
        .record_log_monitor_failure("EE.log could not be read")
        .unwrap();

    assert_eq!(view.health().game_reader().state(), HealthState::Ready);
    assert_eq!(view.health().acquisition_stages().len(), 5);
    assert!(
        view.health()
            .acquisition_stages()
            .iter()
            .all(|stage| stage.state() == HealthState::Ready)
    );
    assert_eq!(view.health().log_monitor().state(), HealthState::Failed);
    assert!(view.health().log_monitor().message().contains("EE.log"));
}

#[test]
fn catalog_without_a_load_keeps_the_prior_fetch_stamp() {
    let mut core = AppCore::in_memory().unwrap();
    let meta = SnapshotMeta::new("123".into(), "build".into(), "warframe-memory".into()).unwrap();
    let result =
        AcquisitionResult::new(snapshot("prior"), AcquisitionHealth::successful()).unwrap();
    let view = core
        .finish_inventory_refresh(Ok((result, meta)), Some((CatalogLoadSource::Network, 100)))
        .unwrap();
    assert_eq!(view.health().catalog().last_success(), Some("100"));

    let result =
        AcquisitionResult::new(snapshot("prior"), AcquisitionHealth::successful()).unwrap();
    let meta = SnapshotMeta::new("123".into(), "build".into(), "warframe-memory".into()).unwrap();
    let view = core
        .finish_inventory_refresh(Ok((result, meta)), None)
        .unwrap();
    assert_eq!(view.health().catalog().state(), HealthState::Degraded);
    assert!(
        view.health().catalog().message().contains("unavailable"),
        "no catalog load reads as unavailable"
    );
    assert_eq!(
        view.health().catalog().last_success(),
        Some("100"),
        "a refresh with no catalog keeps the last fetched stamp"
    );
}
