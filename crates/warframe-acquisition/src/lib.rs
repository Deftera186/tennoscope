#![forbid(unsafe_code)]

use std::{error::Error, fmt};

use warframe_domain::InventorySnapshot;
use zeroize::Zeroizing;

mod authorization;
mod catalog;
mod catalog_cache;
mod collection_prices;
mod inventory;
#[cfg(target_os = "linux")]
mod linux_proc;
mod market;
mod orchestrator;
mod relic_catalog;
mod reward_memory;
mod reward_ui_memory;
#[cfg(windows)]
mod windows_proc;

pub use authorization::AuthorizationScanner;
pub use catalog::{
    CatalogError, CatalogIndex, CatalogMetadata, RewardCatalogEntry, reward_name_matches,
};
pub use catalog_cache::{
    CatalogCache, CatalogCacheError, CatalogFetch, CatalogLoad, CatalogLoadSource, CatalogSource,
    RelicCatalogCache, RelicCatalogLoad, RelicCatalogSource, WFCD_ALL_JSON_URL,
    WFCD_RELICS_JSON_URL, WfcdCatalogHttp, WfcdRelicCatalogHttp,
};
pub use collection_prices::{
    CollectionPriceCache, CollectionPriceSource, DUMP_LOOKBACK_DAYS, MAX_DUMP_BYTES,
    PriceDumpError, PriceFetch, PriceTable, RELICS_RUN_HISTORY_URL, RelicsRunHttp, civil_date,
    dump_is_current, latest_dump, relic_base,
};
pub use inventory::{
    INVENTORY_ENDPOINT, InventoryHttpTransport, InventoryJsonDecoder, MAX_INVENTORY_RESPONSE_BYTES,
};
#[cfg(target_os = "linux")]
pub use linux_proc::LinuxProc;
pub use market::{
    MARKET_MIN_GAP, MarketPriceCache, MarketPriceSource, PriceLookup, RequestPacer,
    WarframeMarketHttp, WarmOutcome, lowest_sell_top, market_slug, slug_and_subtype,
};
pub use orchestrator::{AcquisitionFailure, InventoryAcquirer};
pub use relic_catalog::RelicRewardIndex;
pub use reward_memory::{
    RewardFingerprint, RewardHit, RewardMemoryScanner, RewardNeedle, RewardRepresentation,
    RewardResolution, resolve_current_reward_choices, resolve_reward_choices,
};
pub use reward_ui_memory::PersistentRewardResolver;
#[cfg(windows)]
pub use windows_proc::WindowsProc;

/// A credential whose standard formatting surfaces never expose its contents.
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
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

    fn from_zeroizing(account_id: Zeroizing<String>, nonce: Zeroizing<String>) -> Self {
        Self {
            account_id: SecretString(account_id),
            nonce: SecretString(nonce),
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GameProcess {
    pid: u32,
    start_time_ticks: Option<u64>,
}

impl GameProcess {
    pub const fn new(pid: u32) -> Self {
        Self {
            pid,
            start_time_ticks: None,
        }
    }

    pub const fn pid(self) -> u32 {
        self.pid
    }

    pub(crate) const fn identified(pid: u32, start_time_ticks: u64) -> Self {
        Self {
            pid,
            start_time_ticks: Some(start_time_ticks),
        }
    }

    /// Only the procfs backend reads this back: on Windows the live `PROCESS_VM_READ` handle is
    /// what pins the PID, so the value is recorded for identity but never compared.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) const fn start_time_ticks(self) -> Option<u64> {
        self.start_time_ticks
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RegionScanPriority {
    FileBacked,
    Anonymous,
    WritablePrivateFileBacked,
    WritableAnonymous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadableRegion {
    start: u64,
    len: usize,
    scan_priority: RegionScanPriority,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemorySnapshotRegion {
    start: u64,
    bytes: Vec<u8>,
    scan_priority: RegionScanPriority,
}

impl MemorySnapshotRegion {
    pub fn new(start: u64, bytes: Vec<u8>, scan_priority: RegionScanPriority) -> Self {
        Self {
            start,
            bytes,
            scan_priority,
        }
    }

    pub const fn start(&self) -> u64 {
        self.start
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub const fn scan_priority(&self) -> RegionScanPriority {
        self.scan_priority
    }
}

impl ReadableRegion {
    pub const fn new(start: u64, len: usize) -> Self {
        Self::classified(start, len, RegionScanPriority::Anonymous)
    }

    pub const fn classified(start: u64, len: usize, scan_priority: RegionScanPriority) -> Self {
        Self {
            start,
            len,
            scan_priority,
        }
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

    pub const fn scan_priority(self) -> RegionScanPriority {
        self.scan_priority
    }
}

pub trait ProcessDiscovery {
    fn discover(&self) -> Result<Option<GameProcess>, AcquisitionError>;

    /// Whether the Warframe launcher (not the game itself) is currently visible. Only meaningful
    /// when `discover` returns `Ok(None)` -- it exists to tell "waiting for the launcher" apart
    /// from "nothing is open at all". Defaults to `false` since only the Linux backend can tell.
    fn launcher_present(&self) -> bool {
        false
    }
}
impl<T: ProcessDiscovery + ?Sized> ProcessDiscovery for &T {
    fn discover(&self) -> Result<Option<GameProcess>, AcquisitionError> {
        (**self).discover()
    }
    fn launcher_present(&self) -> bool {
        (**self).launcher_present()
    }
}

pub trait MemoryReader {
    fn readable_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError>;

    fn read_at(
        &self,
        process: &GameProcess,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<usize, AcquisitionError>;

    fn reset_recent_writes(&self, _process: &GameProcess) -> Result<(), AcquisitionError> {
        Ok(())
    }

    fn recently_written_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        self.readable_regions(process)
    }

    fn recently_written_snapshot(
        &self,
        _process: &GameProcess,
    ) -> Result<Option<Vec<MemorySnapshotRegion>>, AcquisitionError> {
        Ok(None)
    }

    fn readable_region_containing(
        &self,
        process: &GameProcess,
        address: u64,
    ) -> Result<Option<ReadableRegion>, AcquisitionError> {
        Ok(self.readable_regions(process)?.into_iter().find(|region| {
            address >= region.start()
                && address < region.start().saturating_add(region.len() as u64)
        }))
    }
}
impl<T: MemoryReader + ?Sized> MemoryReader for &T {
    fn readable_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        (**self).readable_regions(process)
    }
    fn read_at(
        &self,
        process: &GameProcess,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<usize, AcquisitionError> {
        (**self).read_at(process, address, buffer)
    }
    fn reset_recent_writes(&self, process: &GameProcess) -> Result<(), AcquisitionError> {
        (**self).reset_recent_writes(process)
    }
    fn recently_written_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        (**self).recently_written_regions(process)
    }
    fn recently_written_snapshot(
        &self,
        process: &GameProcess,
    ) -> Result<Option<Vec<MemorySnapshotRegion>>, AcquisitionError> {
        (**self).recently_written_snapshot(process)
    }
    fn readable_region_containing(
        &self,
        process: &GameProcess,
        address: u64,
    ) -> Result<Option<ReadableRegion>, AcquisitionError> {
        (**self).readable_region_containing(process, address)
    }
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
    LauncherRunning,
    ProcessDiscoveryFailed,
    MemoryPermissionDenied { pid: u32 },
    MemoryReadFailed { pid: u32 },
    ProcessExited { pid: u32 },
    AuthorizationNotFound,
    AuthorizationAmbiguous,
    InventoryRequestFailed,
    InventoryResponseTooLarge,
    SnapshotInvalid,
    UnsuccessfulHealth,
}

impl fmt::Display for AcquisitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GameNotRunning => formatter.write_str("Warframe is not running"),
            Self::LauncherRunning => {
                formatter.write_str("Warframe launcher is open — waiting for the game to launch")
            }
            Self::ProcessDiscoveryFailed => {
                formatter.write_str("Warframe process discovery failed")
            }
            Self::MemoryPermissionDenied { pid } => {
                write!(
                    formatter,
                    "permission denied while reading Warframe process {pid}; run the helper as the same user/UID and check Yama ptrace settings and sandbox permissions"
                )
            }
            Self::MemoryReadFailed { pid } => {
                write!(formatter, "failed to read Warframe process {pid}")
            }
            Self::ProcessExited { pid } => {
                write!(
                    formatter,
                    "Warframe process {pid} exited during memory acquisition"
                )
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
            Self::UnsuccessfulHealth => {
                formatter.write_str("a successful acquisition cannot contain failed health")
            }
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
    Idle,
    Degraded,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcquisitionDiagnostic {
    Ready,
    GameNotRunning,
    LauncherRunning,
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
            Self::LauncherRunning => "Warframe launcher is open — waiting for the game to launch",
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
    pub const fn ready(stage: AcquisitionStage) -> Self {
        Self {
            stage,
            state: StageState::Ready,
            diagnostic: AcquisitionDiagnostic::Ready,
        }
    }

    pub const fn for_diagnostic(diagnostic: AcquisitionDiagnostic) -> Option<Self> {
        let (stage, state) = match diagnostic {
            AcquisitionDiagnostic::Ready => return None,
            AcquisitionDiagnostic::GameNotRunning | AcquisitionDiagnostic::LauncherRunning => {
                (AcquisitionStage::GameDiscovery, StageState::Idle)
            }
            AcquisitionDiagnostic::ProcessDiscoveryFailed => {
                (AcquisitionStage::GameDiscovery, StageState::Failed)
            }
            AcquisitionDiagnostic::MemoryPermissionDenied
            | AcquisitionDiagnostic::MemoryReadFailed => {
                (AcquisitionStage::MemoryPermission, StageState::Failed)
            }
            AcquisitionDiagnostic::AuthorizationNotFound => (
                AcquisitionStage::AuthorizationDiscovery,
                StageState::Degraded,
            ),
            AcquisitionDiagnostic::AuthorizationAmbiguous => {
                (AcquisitionStage::AuthorizationDiscovery, StageState::Failed)
            }
            AcquisitionDiagnostic::InventoryRequestFailed
            | AcquisitionDiagnostic::InventoryResponseTooLarge => {
                (AcquisitionStage::EndpointFetch, StageState::Failed)
            }
            AcquisitionDiagnostic::SnapshotInvalid => {
                (AcquisitionStage::SchemaValidation, StageState::Failed)
            }
        };
        Some(Self {
            stage,
            state,
            diagnostic,
        })
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

    pub fn successful() -> Self {
        const REQUIRED_STAGES: [AcquisitionStage; 5] = [
            AcquisitionStage::GameDiscovery,
            AcquisitionStage::MemoryPermission,
            AcquisitionStage::AuthorizationDiscovery,
            AcquisitionStage::EndpointFetch,
            AcquisitionStage::SchemaValidation,
        ];
        Self {
            stages: REQUIRED_STAGES
                .into_iter()
                .map(StageHealth::ready)
                .collect(),
        }
    }

    pub fn stages(&self) -> &[StageHealth] {
        &self.stages
    }

    pub fn has_failed_stage(&self) -> bool {
        self.stages
            .iter()
            .any(|stage| stage.state == StageState::Failed)
    }

    pub fn is_successful(&self) -> bool {
        self == &Self::successful()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcquisitionResult {
    snapshot: InventorySnapshot,
    health: AcquisitionHealth,
}

impl AcquisitionResult {
    pub fn new(
        snapshot: InventorySnapshot,
        health: AcquisitionHealth,
    ) -> Result<Self, AcquisitionError> {
        if !health.is_successful() {
            return Err(AcquisitionError::UnsuccessfulHealth);
        }
        Ok(Self { snapshot, health })
    }

    pub fn snapshot(&self) -> &InventorySnapshot {
        &self.snapshot
    }

    pub fn health(&self) -> &AcquisitionHealth {
        &self.health
    }
}
