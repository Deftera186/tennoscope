use std::{collections::BTreeMap, time::Duration};

use warframe_acquisition::{
    AcquisitionError, GameProcess, MemoryReader, PersistentRewardResolver, ReadableRegion,
    RegionScanPriority, RewardNeedle, RewardResolution,
};

struct FixtureMemory {
    regions: Vec<ReadableRegion>,
    bytes: BTreeMap<u64, Vec<u8>>,
}

impl MemoryReader for FixtureMemory {
    fn readable_regions(
        &self,
        _process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        Ok(self.regions.clone())
    }

    fn read_at(
        &self,
        _process: &GameProcess,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<usize, AcquisitionError> {
        let Some((start, bytes)) = self
            .bytes
            .range(..=address)
            .next_back()
            .filter(|(start, bytes)| address < **start + bytes.len() as u64)
        else {
            return Ok(0);
        };
        let offset = usize::try_from(address - *start).unwrap();
        let len = (bytes.len() - offset).min(buffer.len());
        buffer[..len].copy_from_slice(&bytes[offset..offset + len]);
        Ok(len)
    }
}

struct SnapshotTrapMemory(FixtureMemory);

impl MemoryReader for SnapshotTrapMemory {
    fn readable_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        self.0.readable_regions(process)
    }

    fn recently_written_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        self.0.readable_regions(process)
    }

    fn recently_written_snapshot(
        &self,
        _process: &GameProcess,
    ) -> Result<Option<Vec<warframe_acquisition::MemorySnapshotRegion>>, AcquisitionError> {
        panic!("persistent resolver must not eagerly snapshot unbounded dirty memory")
    }

    fn read_at(
        &self,
        process: &GameProcess,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<usize, AcquisitionError> {
        self.0.read_at(process, address, buffer)
    }
}

struct StaleShallowContainerMemory {
    fixture: FixtureMemory,
    stale_fields: Vec<u64>,
}

impl MemoryReader for StaleShallowContainerMemory {
    fn readable_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        self.fixture.readable_regions(process)
    }

    fn recently_written_regions(
        &self,
        process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        self.fixture.readable_regions(process)
    }

    fn read_at(
        &self,
        process: &GameProcess,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<usize, AcquisitionError> {
        if buffer.len() == 8 && self.stale_fields.contains(&address) {
            buffer.fill(0);
            return Ok(buffer.len());
        }
        self.fixture.read_at(process, address, buffer)
    }
}

fn needle(name: &str) -> RewardNeedle {
    RewardNeedle::from_paths(
        name,
        vec![format!(
            "/Lotus/Types/Recipes/Weapons/{}",
            name.replace(' ', "")
        )],
    )
    .unwrap()
}

#[test]
fn resolves_an_ordered_persistent_container_through_intermediate_objects() {
    let choices = [
        "Braton Prime Blueprint",
        "Tipedo Prime Blueprint",
        "Lex Prime Blueprint",
        "Lex Prime Blueprint",
    ];
    let candidates = choices[..3]
        .iter()
        .map(|name| needle(name))
        .collect::<Vec<_>>();
    let base = 0x1000_u64;
    let mut bytes = vec![0_u8; 0x7000];

    let text_addresses = [0x1800_u64, 0x1a00, 0x1c00];
    for (address, name) in text_addresses.iter().zip(choices[..3].iter()) {
        let offset = usize::try_from(*address - base).unwrap();
        bytes[offset..offset + name.len()].copy_from_slice(name.as_bytes());
    }

    let string_objects = text_addresses.map(|address| address - 24);
    let child_objects = [0x3000_u64, 0x3200, 0x3400, 0x3600];
    let child_rewards = [0_usize, 1, 2, 2];
    for (child, reward) in child_objects.iter().zip(child_rewards) {
        let field = usize::try_from(*child + 16 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&string_objects[reward].to_le_bytes());
    }

    let container = 0x5000_u64;
    for (slot, child) in child_objects.iter().enumerate() {
        let field = usize::try_from(container + 64 + (slot as u64 * 8) - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&child.to_le_bytes());
    }

    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            base,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(base, bytes)]),
    };

    assert_eq!(
        PersistentRewardResolver::new(512, 128 * 1024, Duration::from_secs(1))
            .resolve(&memory, &GameProcess::new(7), &candidates, choices.len(),)
            .unwrap(),
        RewardResolution::Confirmed {
            choices: choices.iter().map(|choice| (*choice).into()).collect(),
            region_start: base,
        }
    );
}

#[test]
fn uses_budgeted_region_reads_instead_of_an_eager_dirty_snapshot() {
    let name = "Braton Prime Blueprint";
    let base = 0x1000_u64;
    let mut bytes = vec![0_u8; 4096];
    bytes[512..512 + name.len()].copy_from_slice(name.as_bytes());
    let memory = SnapshotTrapMemory(FixtureMemory {
        regions: vec![ReadableRegion::classified(
            base,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(base, bytes)]),
    });

    assert_eq!(
        PersistentRewardResolver::new(512, 128 * 1024, Duration::from_secs(1))
            .resolve(&memory, &GameProcess::new(7), &[needle(name)], 4)
            .unwrap(),
        RewardResolution::Incomplete
    );
}

#[test]
fn rejects_candidate_strings_without_an_ordered_pointer_container() {
    let name = "Braton Prime Blueprint";
    let base = 0x1000_u64;
    let mut bytes = vec![0_u8; 4096];
    bytes[512..512 + name.len()].copy_from_slice(name.as_bytes());
    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            base,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(base, bytes)]),
    };

    assert_eq!(
        PersistentRewardResolver::new(512, 128 * 1024, Duration::from_secs(1))
            .resolve(&memory, &GameProcess::new(7), &[needle(name)], 4,)
            .unwrap(),
        RewardResolution::Incomplete
    );
}

#[test]
fn rejects_speculative_non_string_object_offsets() {
    let names = ["Paris Prime Lower Limb", "Vasto Prime Blueprint"];
    let candidates = names.iter().map(|name| needle(name)).collect::<Vec<_>>();
    let base = 0x1000_u64;
    let mut bytes = vec![0_u8; 0x6000];
    let text_addresses = [0x1800_u64, 0x1a00];
    for (address, name) in text_addresses.iter().zip(names) {
        let offset = usize::try_from(*address - base).unwrap();
        bytes[offset..offset + name.len()].copy_from_slice(name.as_bytes());
    }
    let fake_children = [0x3000_u64, 0x3400];
    for (child, text) in fake_children.iter().zip(text_addresses) {
        let field = usize::try_from(*child + 16 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&(text - 16).to_le_bytes());
    }
    let fake_container = 0x5000_u64;
    for (slot, child) in fake_children.iter().enumerate() {
        let field = usize::try_from(fake_container + 64 + slot as u64 * 8 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&child.to_le_bytes());
    }
    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            base,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(base, bytes)]),
    };

    assert_eq!(
        PersistentRewardResolver::new(512, 128 * 1024, Duration::from_secs(1))
            .resolve(&memory, &GameProcess::new(7), &candidates, 2)
            .unwrap(),
        RewardResolution::Incomplete
    );
}

#[test]
fn rejects_competing_ordered_containers() {
    let names = ["Braton Prime Blueprint", "Tipedo Prime Blueprint"];
    let candidates = names.iter().map(|name| needle(name)).collect::<Vec<_>>();
    let base = 0x1000_u64;
    let mut bytes = vec![0_u8; 0x7000];
    let text_addresses = [0x1800_u64, 0x1a00];
    for (address, name) in text_addresses.iter().zip(names) {
        let offset = usize::try_from(*address - base).unwrap();
        bytes[offset..offset + name.len()].copy_from_slice(name.as_bytes());
    }
    let string_objects = text_addresses.map(|address| address - 24);
    let children = [0x3000_u64, 0x3200];
    for (child, string) in children.iter().zip(string_objects) {
        let field = usize::try_from(*child + 16 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&string.to_le_bytes());
    }
    for (container, order) in [(0x5000_u64, [0_usize, 1]), (0x5200, [1_usize, 0])] {
        for (slot, child_index) in order.into_iter().enumerate() {
            let field = usize::try_from(container + 64 + (slot as u64 * 8) - base).unwrap();
            bytes[field..field + 8].copy_from_slice(&children[child_index].to_le_bytes());
        }
    }
    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            base,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(base, bytes)]),
    };

    assert_eq!(
        PersistentRewardResolver::new(512, 128 * 1024, Duration::from_secs(1))
            .resolve(&memory, &GameProcess::new(7), &candidates, 2)
            .unwrap(),
        RewardResolution::Ambiguous
    );
}

#[test]
fn rejects_a_repeated_pointer_to_one_reward_object_as_four_cards() {
    let name = "Vadarya Prime Receiver";
    let base = 0x1000_u64;
    let mut bytes = vec![0_u8; 0x7000];
    let text_address = 0x1800_u64;
    let text_offset = usize::try_from(text_address - base).unwrap();
    bytes[text_offset..text_offset + name.len()].copy_from_slice(name.as_bytes());

    let child = 0x3000_u64;
    let child_field = usize::try_from(child + 16 - base).unwrap();
    bytes[child_field..child_field + 8].copy_from_slice(&(text_address - 24).to_le_bytes());

    let container = 0x5000_u64;
    for slot in 0..4_u64 {
        let field = usize::try_from(container + 64 + slot * 8 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&child.to_le_bytes());
    }

    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            base,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(base, bytes)]),
    };

    assert_eq!(
        PersistentRewardResolver::new(512, 128 * 1024, Duration::from_secs(1))
            .resolve(&memory, &GameProcess::new(7), &[needle(name)], 4)
            .unwrap(),
        RewardResolution::Incomplete
    );
}

#[test]
fn continues_to_a_deeper_container_when_a_shallow_candidate_is_stale() {
    let names = [
        "Forma Blueprint",
        "Xaku Prime Neuroptics Blueprint",
        "Orthos Prime Blade",
        "Lex Prime Barrel",
    ];
    let candidates = names.iter().map(|name| needle(name)).collect::<Vec<_>>();
    let base = 0x1000_u64;
    let mut bytes = vec![0_u8; 0xa000];
    let text_addresses = [0x1800_u64, 0x1a00, 0x1c00, 0x1e00];
    for (address, name) in text_addresses.iter().zip(names) {
        let offset = usize::try_from(*address - base).unwrap();
        bytes[offset..offset + name.len()].copy_from_slice(name.as_bytes());
    }

    let children = [0x3000_u64, 0x3400, 0x3800, 0x3c00];
    for (child, text) in children.iter().zip(text_addresses) {
        let field = usize::try_from(*child + 16 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&(text - 24).to_le_bytes());
    }

    let stale_container = 0x4800_u64;
    let mut stale_fields = Vec::new();
    for (slot, child) in children.iter().enumerate() {
        let address = stale_container + 64 + slot as u64 * 8;
        let field = usize::try_from(address - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&child.to_le_bytes());
        stale_fields.push(address);
    }

    let intermediates = [0x6000_u64, 0x6400, 0x6800, 0x6c00];
    for (intermediate, child) in intermediates.iter().zip(children) {
        let field = usize::try_from(*intermediate + 24 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&child.to_le_bytes());
    }
    let live_container = 0x8000_u64;
    for (slot, intermediate) in intermediates.iter().enumerate() {
        let field = usize::try_from(live_container + 64 + slot as u64 * 8 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&intermediate.to_le_bytes());
    }

    let memory = StaleShallowContainerMemory {
        fixture: FixtureMemory {
            regions: vec![ReadableRegion::classified(
                base,
                bytes.len(),
                RegionScanPriority::WritableAnonymous,
            )],
            bytes: BTreeMap::from([(base, bytes)]),
        },
        stale_fields,
    };

    assert_eq!(
        PersistentRewardResolver::new(512, 256 * 1024, Duration::from_secs(1))
            .resolve(&memory, &GameProcess::new(7), &candidates, 4)
            .unwrap(),
        RewardResolution::Confirmed {
            choices: names.iter().map(|name| (*name).to_owned()).collect(),
            region_start: base,
        }
    );
}

#[test]
fn prefers_the_deeper_card_list_over_a_confirmed_shallow_subobject_array() {
    let names = [
        "2X Forma Blueprint",
        "Vadarya Prime Stock",
        "Dual Zoren Prime Blueprint",
        "Paris Prime Grip",
    ];
    let candidates = names.iter().map(|name| needle(name)).collect::<Vec<_>>();
    let base = 0x1000_u64;
    let mut bytes = vec![0_u8; 0xc000];
    let text_addresses = [0x1800_u64, 0x1a00, 0x1c00, 0x1e00];
    for (address, name) in text_addresses.iter().zip(names) {
        let offset = usize::try_from(*address - base).unwrap();
        bytes[offset..offset + name.len()].copy_from_slice(name.as_bytes());
    }

    let real_children = [0x3000_u64, 0x3400, 0x3800, 0x3c00];
    for (child, text) in real_children.iter().zip(text_addresses) {
        let field = usize::try_from(*child + 16 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&(text - 24).to_le_bytes());
    }

    let forma_subobjects = [0x4400_u64, 0x4800, 0x4c00, 0x5000];
    for subobject in forma_subobjects {
        let field = usize::try_from(subobject + 16 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&(text_addresses[0] - 24).to_le_bytes());
    }
    let shallow = 0x5800_u64;
    for (slot, subobject) in forma_subobjects.iter().enumerate() {
        let field = usize::try_from(shallow + 64 + slot as u64 * 8 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&subobject.to_le_bytes());
    }

    let card_objects = [0x7000_u64, 0x7400, 0x7800, 0x7c00];
    for (card, child) in card_objects.iter().zip(real_children) {
        let field = usize::try_from(*card + 24 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&child.to_le_bytes());
    }
    let card_list = 0x9000_u64;
    for (slot, card) in card_objects.iter().enumerate() {
        let field = usize::try_from(card_list + 64 + slot as u64 * 8 - base).unwrap();
        bytes[field..field + 8].copy_from_slice(&card.to_le_bytes());
    }

    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            base,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(base, bytes)]),
    };

    assert_eq!(
        PersistentRewardResolver::new(512, 256 * 1024, Duration::from_secs(1))
            .resolve(&memory, &GameProcess::new(7), &candidates, 4)
            .unwrap(),
        RewardResolution::Confirmed {
            choices: names.iter().map(|name| (*name).to_owned()).collect(),
            region_start: base,
        }
    );
}
