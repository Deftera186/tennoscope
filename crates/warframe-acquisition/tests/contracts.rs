use std::fmt::Write as _;

use warframe_acquisition::{
    AcquisitionDiagnostic, AcquisitionError, AcquisitionHealth, AcquisitionResult,
    AcquisitionStage, GameProcess, InventoryAuthorization, InventoryTransport, MemoryReader,
    ProcessDiscovery, ReadableRegion, SecretString, SnapshotDecoder, StageHealth, StageState,
};
use warframe_domain::InventorySnapshot;

const ACCOUNT_ID: &str = "account-id-that-must-never-appear";
const NONCE: &str = "987654321012345678";

#[test]
fn secret_debug_and_display_are_fully_redacted() {
    let secret = SecretString::new(ACCOUNT_ID);

    assert_eq!(format!("{secret}"), "[REDACTED]");
    assert_eq!(format!("{secret:?}"), "SecretString([REDACTED])");
    assert!(!format!("{secret}{secret:?}").contains(ACCOUNT_ID));
}

#[test]
fn authorization_debug_never_reveals_its_parts() {
    let authorization = InventoryAuthorization::new(ACCOUNT_ID, NONCE);
    let rendered = format!("{authorization:?}");

    assert!(!rendered.contains(ACCOUNT_ID));
    assert!(!rendered.contains(NONCE));
    assert_eq!(rendered.matches("[REDACTED]").count(), 2);
}

#[test]
fn all_public_errors_and_diagnostics_are_secret_free() {
    let errors = [
        AcquisitionError::GameNotRunning,
        AcquisitionError::ProcessDiscoveryFailed,
        AcquisitionError::MemoryPermissionDenied { pid: 42 },
        AcquisitionError::MemoryReadFailed { pid: 42 },
        AcquisitionError::AuthorizationNotFound,
        AcquisitionError::AuthorizationAmbiguous,
        AcquisitionError::InventoryRequestFailed,
        AcquisitionError::InventoryResponseTooLarge,
        AcquisitionError::SnapshotInvalid,
    ];
    let diagnostics = [
        AcquisitionDiagnostic::GameNotRunning,
        AcquisitionDiagnostic::ProcessDiscoveryFailed,
        AcquisitionDiagnostic::MemoryPermissionDenied,
        AcquisitionDiagnostic::MemoryReadFailed,
        AcquisitionDiagnostic::AuthorizationNotFound,
        AcquisitionDiagnostic::AuthorizationAmbiguous,
        AcquisitionDiagnostic::InventoryRequestFailed,
        AcquisitionDiagnostic::InventoryResponseTooLarge,
        AcquisitionDiagnostic::SnapshotInvalid,
    ];
    let mut rendered = String::new();
    for error in errors {
        write!(&mut rendered, "{error}{error:?}").unwrap();
    }
    for diagnostic in diagnostics {
        write!(&mut rendered, "{diagnostic}{diagnostic:?}").unwrap();
    }

    assert!(!rendered.contains(ACCOUNT_ID));
    assert!(!rendered.contains(NONCE));
}

#[test]
fn acquisition_health_reports_structured_secret_free_stages() {
    let health = AcquisitionHealth::new(vec![StageHealth::new(
        AcquisitionStage::AuthorizationDiscovery,
        StageState::Failed,
        AcquisitionDiagnostic::AuthorizationNotFound,
    )]);

    assert_eq!(health.stages().len(), 1);
    assert_eq!(
        health.stages()[0].stage(),
        AcquisitionStage::AuthorizationDiscovery
    );
    assert_eq!(health.stages()[0].state(), StageState::Failed);
    assert_eq!(
        health.stages()[0].diagnostic(),
        AcquisitionDiagnostic::AuthorizationNotFound
    );
}

#[test]
fn acquisition_result_carries_only_a_validated_snapshot_and_health() {
    let snapshot = InventorySnapshot::coherent(vec![]).unwrap();
    let result = AcquisitionResult::new(snapshot, AcquisitionHealth::new(vec![]));

    assert!(result.snapshot().entries().is_empty());
    assert!(result.health().stages().is_empty());
}

#[allow(dead_code)]
fn contracts_are_object_safe(
    discovery: &dyn ProcessDiscovery,
    memory: &dyn MemoryReader,
    transport: &dyn InventoryTransport,
    decoder: &dyn SnapshotDecoder,
    process: &GameProcess,
    region: ReadableRegion,
    authorization: &InventoryAuthorization,
) {
    let _ = discovery.discover();
    let _ = memory.readable_regions(process);
    let _ = memory.read(process, region);
    let _ = transport.fetch(authorization);
    let _ = decoder.decode(&[]);
}
