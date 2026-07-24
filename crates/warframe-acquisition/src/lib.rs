#![forbid(unsafe_code)]

use std::{error::Error, fmt};

use warframe_domain::InventorySnapshot;

/// A credential whose standard formatting surfaces never expose its contents.
#[derive(Clone, Eq, PartialEq)]
pub struct SecretString(Box<str>);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into().into_boxed_str())
    }

    #[allow(dead_code)] // Used by the scanner and transport in later adapter tasks.
    pub(crate) fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct InventoryAuthorization {
    account_id: SecretString,
    nonce: SecretString,
}

impl InventoryAuthorization {
    pub fn new(account_id: impl Into<String>, nonce: impl Into<String>) -> Self {
        Self {
            account_id: SecretString::new(account_id),
            nonce: SecretString::new(nonce),
        }
    }

    #[allow(dead_code)] // Used by the transport in a later adapter task.
    pub(crate) fn account_id(&self) -> &str {
        self.account_id.expose_secret()
    }

    #[allow(dead_code)] // Used by the transport in a later adapter task.
    pub(crate) fn nonce(&self) -> &str {
        self.nonce.expose_secret()
    }
}

impl fmt::Debug for InventoryAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InventoryAuthorization")
            .field("account_id", &self.account_id)
            .field("nonce", &self.nonce)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GameProcess {
    pid: u32,
}

impl GameProcess {
    pub const fn new(pid: u32) -> Self {
        Self { pid }
    }

    pub const fn pid(self) -> u32 {
        self.pid
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadableRegion {
    start: u64,
    len: usize,
}

impl ReadableRegion {
    pub const fn new(start: u64, len: usize) -> Self {
        Self { start, len }
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn len(self) -> usize {
        self.len
    }

    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

pub trait ProcessDiscovery {
    fn discover(&self) -> Result<Option<GameProcess>, AcquisitionError>;
}

pub trait MemoryReader {
    fn readable_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError>;

    fn read(
        &self,
        process: &GameProcess,
        region: ReadableRegion,
    ) -> Result<Vec<u8>, AcquisitionError>;
}

pub trait InventoryTransport {
    fn fetch(&self, authorization: &InventoryAuthorization) -> Result<Vec<u8>, AcquisitionError>;
}

pub trait SnapshotDecoder {
    fn decode(&self, response: &[u8]) -> Result<InventorySnapshot, AcquisitionError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcquisitionError {
    GameNotRunning,
    ProcessDiscoveryFailed,
    MemoryPermissionDenied { pid: u32 },
    MemoryReadFailed { pid: u32 },
    AuthorizationNotFound,
    AuthorizationAmbiguous,
    InventoryRequestFailed,
    InventoryResponseTooLarge,
    SnapshotInvalid,
}

impl fmt::Display for AcquisitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GameNotRunning => formatter.write_str("Warframe is not running"),
            Self::ProcessDiscoveryFailed => {
                formatter.write_str("Warframe process discovery failed")
            }
            Self::MemoryPermissionDenied { pid } => {
                write!(
                    formatter,
                    "permission denied while reading Warframe process {pid}"
                )
            }
            Self::MemoryReadFailed { pid } => {
                write!(formatter, "failed to read Warframe process {pid}")
            }
            Self::AuthorizationNotFound => {
                formatter.write_str("inventory authorization was not found")
            }
            Self::AuthorizationAmbiguous => {
                formatter.write_str("multiple inventory authorizations were found")
            }
            Self::InventoryRequestFailed => formatter.write_str("inventory request failed"),
            Self::InventoryResponseTooLarge => {
                formatter.write_str("inventory response exceeded the size limit")
            }
            Self::SnapshotInvalid => formatter.write_str("inventory snapshot was invalid"),
        }
    }
}

impl Error for AcquisitionError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcquisitionStage {
    GameDiscovery,
    MemoryPermission,
    AuthorizationDiscovery,
    EndpointFetch,
    SchemaValidation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageState {
    Ready,
    Degraded,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcquisitionDiagnostic {
    Ready,
    GameNotRunning,
    ProcessDiscoveryFailed,
    MemoryPermissionDenied,
    MemoryReadFailed,
    AuthorizationNotFound,
    AuthorizationAmbiguous,
    InventoryRequestFailed,
    InventoryResponseTooLarge,
    SnapshotInvalid,
}

impl fmt::Display for AcquisitionDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Ready => "ready",
            Self::GameNotRunning => "Warframe is not running",
            Self::ProcessDiscoveryFailed => "Warframe process discovery failed",
            Self::MemoryPermissionDenied => "permission to read Warframe memory was denied",
            Self::MemoryReadFailed => "Warframe memory could not be read",
            Self::AuthorizationNotFound => "inventory authorization was not found",
            Self::AuthorizationAmbiguous => "multiple inventory authorizations were found",
            Self::InventoryRequestFailed => "inventory request failed",
            Self::InventoryResponseTooLarge => "inventory response exceeded the size limit",
            Self::SnapshotInvalid => "inventory snapshot was invalid",
        };
        formatter.write_str(message)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StageHealth {
    stage: AcquisitionStage,
    state: StageState,
    diagnostic: AcquisitionDiagnostic,
}

impl StageHealth {
    pub const fn new(
        stage: AcquisitionStage,
        state: StageState,
        diagnostic: AcquisitionDiagnostic,
    ) -> Self {
        Self {
            stage,
            state,
            diagnostic,
        }
    }

    pub const fn stage(self) -> AcquisitionStage {
        self.stage
    }

    pub const fn state(self) -> StageState {
        self.state
    }

    pub const fn diagnostic(self) -> AcquisitionDiagnostic {
        self.diagnostic
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcquisitionHealth {
    stages: Vec<StageHealth>,
}

impl AcquisitionHealth {
    pub fn new(stages: Vec<StageHealth>) -> Self {
        Self { stages }
    }

    pub fn stages(&self) -> &[StageHealth] {
        &self.stages
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcquisitionResult {
    snapshot: InventorySnapshot,
    health: AcquisitionHealth,
}

impl AcquisitionResult {
    pub fn new(snapshot: InventorySnapshot, health: AcquisitionHealth) -> Self {
        Self { snapshot, health }
    }

    pub fn snapshot(&self) -> &InventorySnapshot {
        &self.snapshot
    }

    pub fn health(&self) -> &AcquisitionHealth {
        &self.health
    }
}
