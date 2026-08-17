use std::cell::Cell;

use warframe_acquisition::{
    AcquisitionError, AcquisitionStage, CatalogIndex, GameProcess, InventoryAcquirer,
    InventoryAuthorization, InventoryTransport, MemoryReader, ProcessDiscovery, ReadableRegion,
    RegionScanPriority, StageState,
};

struct Discovery(Result<Option<GameProcess>, AcquisitionError>);
impl ProcessDiscovery for Discovery {
    fn discover(&self) -> Result<Option<GameProcess>, AcquisitionError> {
        self.0
    }
}

struct LauncherOnlyDiscovery;
impl ProcessDiscovery for LauncherOnlyDiscovery {
    fn discover(&self) -> Result<Option<GameProcess>, AcquisitionError> {
        Ok(None)
    }
    fn launcher_present(&self) -> bool {
        true
    }
}

struct Memory(Vec<u8>);
impl MemoryReader for Memory {
    fn readable_regions(&self, _: &GameProcess) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        Ok(vec![ReadableRegion::classified(
            0x1000,
            self.0.len(),
            RegionScanPriority::WritableAnonymous,
        )])
    }
    fn read_at(
        &self,
        _: &GameProcess,
        address: u64,
        output: &mut [u8],
    ) -> Result<usize, AcquisitionError> {
        let start = usize::try_from(address - 0x1000).unwrap();
        let count = output.len().min(self.0.len().saturating_sub(start));
        output[..count].copy_from_slice(&self.0[start..start + count]);
        Ok(count)
    }
}

struct Transport {
    calls: Cell<usize>,
    body: Result<Vec<u8>, AcquisitionError>,
}
impl InventoryTransport for Transport {
    fn fetch(&self, _: &InventoryAuthorization) -> Result<Vec<u8>, AcquisitionError> {
        self.calls.set(self.calls.get() + 1);
        self.body.clone()
    }
}

fn auth_memory() -> Vec<u8> {
    let fixture = include_bytes!("fixtures/authorization-url-encoded.bin");
    [fixture.as_slice(), fixture.as_slice(), fixture.as_slice()].concat()
}

fn inventory() -> Vec<u8> {
    br#"{"LastInventorySync":1,"Suits":[{"ItemType":"/Lotus/Powersuits/Test/Test"}],"LongGuns":[],"Pistols":[],"Melee":[],"Sentinels":[],"MiscItems":[],"Recipes":[],"PendingRecipes":[],"XPInfo":[],"SpaceSuits":[],"SpaceMelee":[],"SpaceGuns":[],"SentinelWeapons":[],"KubrowPets":[],"OperatorAmps":[],"MechSuits":[]}"#.to_vec()
}

fn catalog() -> CatalogIndex {
    CatalogIndex::from_wfcd_json(br#"[{"uniqueName":"/Lotus/Powersuits/Test/Test","name":"Test Frame","type":"Warframe","category":"Warframes","masterable":true}]"#).unwrap()
}

#[test]
fn acquires_one_coherent_snapshot_through_all_stages() {
    let transport = Transport {
        calls: Cell::new(0),
        body: Ok(inventory()),
    };
    let acquirer = InventoryAcquirer::new(
        Discovery(Ok(Some(GameProcess::new(7)))),
        Memory(auth_memory()),
        transport,
    );
    let result = acquirer.acquire(&catalog()).unwrap();
    assert_eq!(result.snapshot().entries().len(), 1);
    assert_eq!(result.snapshot().entries()[0].item.name, "Test Frame");
    assert!(result.health().is_successful());
}

#[test]
fn failure_reports_the_exact_stage_and_does_not_fetch_later_stages() {
    let transport = Transport {
        calls: Cell::new(0),
        body: Ok(inventory()),
    };
    let acquirer = InventoryAcquirer::new(Discovery(Ok(None)), Memory(vec![]), transport);
    let failure = acquirer.acquire(&catalog()).unwrap_err();
    assert_eq!(failure.error(), AcquisitionError::GameNotRunning);
    assert_eq!(
        failure.health().stages()[0].stage(),
        AcquisitionStage::GameDiscovery
    );
    assert_eq!(failure.health().stages()[0].state(), StageState::Degraded);
    assert_eq!(acquirer.transport().calls.get(), 0);
    assert!(!format!("{failure:?}").contains("0123456789abcdef"));
}

#[test]
fn a_launcher_seen_through_a_reference_is_reported_rather_than_a_bare_absence() {
    let discovery = LauncherOnlyDiscovery;
    let transport = Transport {
        calls: Cell::new(0),
        body: Ok(inventory()),
    };
    // By reference, not owned: this is what exercises the blanket `&T` `ProcessDiscovery` impl,
    // which is the one place `launcher_present` could silently fall back to its default.
    let acquirer = InventoryAcquirer::new(&discovery, Memory(vec![]), transport);

    let failure = acquirer.acquire(&catalog()).unwrap_err();

    assert_eq!(failure.error(), AcquisitionError::LauncherRunning);
}
