use crate::{
    AcquisitionDiagnostic, AcquisitionError, AcquisitionHealth, AcquisitionResult,
    AuthorizationScanner, CatalogIndex, InventoryJsonDecoder, InventoryTransport, MemoryReader,
    ProcessDiscovery, SnapshotDecoder, StageHealth,
};

const SCAN_CHUNK_BYTES: usize = 1024 * 1024;

pub struct InventoryAcquirer<D, M, T> {
    discovery: D,
    memory: M,
    transport: T,
}

impl<D, M, T> InventoryAcquirer<D, M, T>
where
    D: ProcessDiscovery,
    M: MemoryReader,
    T: InventoryTransport,
{
    pub const fn new(discovery: D, memory: M, transport: T) -> Self {
        Self {
            discovery,
            memory,
            transport,
        }
    }

    pub fn acquire(&self, catalog: &CatalogIndex) -> Result<AcquisitionResult, AcquisitionFailure> {
        let process = self
            .discovery
            .discover()
            .map_err(AcquisitionFailure::from_error)?
            .ok_or_else(|| {
                let error = if self.discovery.launcher_present() {
                    AcquisitionError::LauncherRunning
                } else {
                    AcquisitionError::GameNotRunning
                };
                AcquisitionFailure::from_error(error)
            })?;
        log::info!("acquisition: process discovered pid={}", process.pid());
        let authorization = AuthorizationScanner::new(SCAN_CHUNK_BYTES)
            .scan(&self.memory, &process)
            .map_err(AcquisitionFailure::from_error)?;
        log::info!("acquisition: authorization scan chunks=1 bytes={SCAN_CHUNK_BYTES}");
        let body = self
            .transport
            .fetch(&authorization)
            .map_err(AcquisitionFailure::from_error)?;
        log::info!("acquisition: fetch ok bytes={}", body.len());
        let snapshot = InventoryJsonDecoder::with_catalog(catalog)
            .decode(&body)
            .map_err(AcquisitionFailure::from_error)?;
        log::info!(
            "acquisition: decode ok entries={}",
            snapshot.entries().len()
        );
        AcquisitionResult::new(snapshot, AcquisitionHealth::successful())
            .map_err(AcquisitionFailure::from_error)
    }

    pub const fn transport(&self) -> &T {
        &self.transport
    }
}

#[derive(Clone)]
pub struct AcquisitionFailure {
    error: AcquisitionError,
    health: AcquisitionHealth,
}

impl AcquisitionFailure {
    pub fn from_error(error: AcquisitionError) -> Self {
        let diagnostic = match error {
            AcquisitionError::GameNotRunning => AcquisitionDiagnostic::GameNotRunning,
            AcquisitionError::LauncherRunning => AcquisitionDiagnostic::LauncherRunning,
            AcquisitionError::ProcessDiscoveryFailed | AcquisitionError::ProcessExited { .. } => {
                AcquisitionDiagnostic::ProcessDiscoveryFailed
            }
            AcquisitionError::MemoryPermissionDenied { .. } => {
                AcquisitionDiagnostic::MemoryPermissionDenied
            }
            AcquisitionError::MemoryReadFailed { .. } => AcquisitionDiagnostic::MemoryReadFailed,
            AcquisitionError::AuthorizationNotFound => AcquisitionDiagnostic::AuthorizationNotFound,
            AcquisitionError::AuthorizationAmbiguous => {
                AcquisitionDiagnostic::AuthorizationAmbiguous
            }
            AcquisitionError::InventoryRequestFailed => {
                AcquisitionDiagnostic::InventoryRequestFailed
            }
            AcquisitionError::InventoryResponseTooLarge => {
                AcquisitionDiagnostic::InventoryResponseTooLarge
            }
            AcquisitionError::SnapshotInvalid | AcquisitionError::UnsuccessfulHealth => {
                AcquisitionDiagnostic::SnapshotInvalid
            }
        };
        let stage =
            StageHealth::for_diagnostic(diagnostic).expect("failure diagnostic has a stage");
        Self {
            error,
            health: AcquisitionHealth::new(vec![stage]),
        }
    }

    pub const fn error(&self) -> AcquisitionError {
        self.error
    }
    pub const fn health(&self) -> &AcquisitionHealth {
        &self.health
    }

    #[doc(hidden)]
    pub fn for_test(error: AcquisitionError) -> Self {
        Self::from_error(error)
    }
}

impl std::fmt::Debug for AcquisitionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcquisitionFailure")
            .field("error", &self.error)
            .field("health", &self.health)
            .finish()
    }
}

impl std::fmt::Display for AcquisitionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for AcquisitionFailure {}
