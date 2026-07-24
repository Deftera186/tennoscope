use std::fmt::Write as _;

use static_assertions::assert_not_impl_any;
use warframe_acquisition::{
    AcquisitionDiagnostic, AcquisitionError, AcquisitionHealth, AcquisitionResult,
    AcquisitionStage, GameProcess, InventoryAuthorization, InventoryTransport, MemoryReader,
    ProcessDiscovery, ReadableRegion, SecretString, SnapshotDecoder, StageHealth, StageState,
};
use warframe_domain::InventorySnapshot;

const ACCOUNT_ID: &str = "account-id-that-must-never-appear";
const NONCE: &str = "987654321012345678";

assert_not_impl_any!(SecretString: Clone, Copy);
assert_not_impl_any!(InventoryAuthorization: Clone, Copy);

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
        AcquisitionError::ProcessExited { pid: 42 },
        AcquisitionError::AuthorizationNotFound,
        AcquisitionError::AuthorizationAmbiguous,
        AcquisitionError::InventoryRequestFailed,
        AcquisitionError::InventoryResponseTooLarge,
        AcquisitionError::SnapshotInvalid,
        AcquisitionError::UnsuccessfulHealth,
    ];
    let diagnostics = [
        AcquisitionDiagnostic::Ready,
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
    let health = AcquisitionHealth::new(vec![
        StageHealth::for_diagnostic(AcquisitionDiagnostic::AuthorizationNotFound).unwrap(),
    ]);

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
    let health = AcquisitionHealth::successful();
    let result = AcquisitionResult::new(snapshot, health).unwrap();

    assert!(result.snapshot().entries().is_empty());
    assert_eq!(result.health().stages().len(), 5);
    assert!(
        result
            .health()
            .stages()
            .iter()
            .all(|stage| stage.state() == StageState::Ready)
    );
}

#[test]
fn diagnostic_selects_its_canonical_stage_and_state() {
    let health =
        StageHealth::for_diagnostic(AcquisitionDiagnostic::MemoryPermissionDenied).unwrap();

    assert_eq!(health.stage(), AcquisitionStage::MemoryPermission);
    assert_eq!(health.state(), StageState::Failed);
    assert_eq!(
        health.diagnostic(),
        AcquisitionDiagnostic::MemoryPermissionDenied
    );
}

#[test]
fn successful_result_rejects_failed_health() {
    let snapshot = InventorySnapshot::coherent(vec![]).unwrap();
    let health = AcquisitionHealth::new(vec![
        StageHealth::for_diagnostic(AcquisitionDiagnostic::SnapshotInvalid).unwrap(),
    ]);

    assert_eq!(
        AcquisitionResult::new(snapshot, health).unwrap_err(),
        AcquisitionError::UnsuccessfulHealth
    );
}

#[test]
fn successful_result_rejects_degraded_health() {
    let snapshot = InventorySnapshot::coherent(vec![]).unwrap();
    let health = AcquisitionHealth::new(vec![
        StageHealth::for_diagnostic(AcquisitionDiagnostic::GameNotRunning).unwrap(),
    ]);

    assert_eq!(
        AcquisitionResult::new(snapshot, health).unwrap_err(),
        AcquisitionError::UnsuccessfulHealth
    );
}

#[test]
fn successful_result_rejects_empty_or_missing_health() {
    let empty_snapshot = InventorySnapshot::coherent(vec![]).unwrap();
    assert_eq!(
        AcquisitionResult::new(empty_snapshot, AcquisitionHealth::new(vec![])).unwrap_err(),
        AcquisitionError::UnsuccessfulHealth
    );

    let missing_snapshot = InventorySnapshot::coherent(vec![]).unwrap();
    let missing = AcquisitionHealth::new(vec![StageHealth::ready(AcquisitionStage::GameDiscovery)]);
    assert_eq!(
        AcquisitionResult::new(missing_snapshot, missing).unwrap_err(),
        AcquisitionError::UnsuccessfulHealth
    );
}

#[test]
fn successful_result_rejects_duplicate_or_out_of_order_stages() {
    let duplicate_snapshot = InventorySnapshot::coherent(vec![]).unwrap();
    let duplicate = AcquisitionHealth::new(vec![
        StageHealth::ready(AcquisitionStage::GameDiscovery),
        StageHealth::ready(AcquisitionStage::GameDiscovery),
        StageHealth::ready(AcquisitionStage::AuthorizationDiscovery),
        StageHealth::ready(AcquisitionStage::EndpointFetch),
        StageHealth::ready(AcquisitionStage::SchemaValidation),
    ]);
    assert_eq!(
        AcquisitionResult::new(duplicate_snapshot, duplicate).unwrap_err(),
        AcquisitionError::UnsuccessfulHealth
    );

    let out_of_order_snapshot = InventorySnapshot::coherent(vec![]).unwrap();
    let out_of_order = AcquisitionHealth::new(vec![
        StageHealth::ready(AcquisitionStage::MemoryPermission),
        StageHealth::ready(AcquisitionStage::GameDiscovery),
        StageHealth::ready(AcquisitionStage::AuthorizationDiscovery),
        StageHealth::ready(AcquisitionStage::EndpointFetch),
        StageHealth::ready(AcquisitionStage::SchemaValidation),
    ]);
    assert_eq!(
        AcquisitionResult::new(out_of_order_snapshot, out_of_order).unwrap_err(),
        AcquisitionError::UnsuccessfulHealth
    );
}

#[test]
fn memory_reader_performs_bounded_partial_reads_into_caller_buffer() {
    struct PartialReader;

    impl MemoryReader for PartialReader {
        fn readable_regions(
            &self,
            _process: &GameProcess,
        ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
            Ok(vec![ReadableRegion::new(0x1000, 128)])
        }

        fn read_at(
            &self,
            _process: &GameProcess,
            address: u64,
            buffer: &mut [u8],
        ) -> Result<usize, AcquisitionError> {
            assert_eq!(address, 0x1004);
            let source = b"partial";
            let read = source.len().min(buffer.len());
            buffer[..read].copy_from_slice(&source[..read]);
            Ok(read)
        }
    }

    let mut buffer = [0_u8; 4];
    let read = PartialReader
        .read_at(&GameProcess::new(7), 0x1004, &mut buffer)
        .unwrap();

    assert_eq!(read, 4);
    assert_eq!(&buffer, b"part");
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
    let mut buffer = [0_u8; 32];
    let _ = memory.read_at(process, region.start(), &mut buffer);
    let _ = transport.fetch(authorization);
    let _ = decoder.decode(&[]);
}
