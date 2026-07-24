use local_store::SnapshotMeta;
use warframe_domain::{
    CatalogItem, Category, InventoryEntry, InventorySnapshot, ItemId, RewardCandidate,
};

use crate::AppError;

pub(crate) struct FakeSession {
    pub(crate) snapshot: InventorySnapshot,
    pub(crate) meta: SnapshotMeta,
    pub(crate) rewards: Vec<RewardCandidate>,
}

pub(crate) fn build() -> Result<FakeSession, AppError> {
    let snapshot = InventorySnapshot::coherent(vec![
        entry(
            "saryn-prime-chassis",
            "Saryn Prime Chassis",
            Category::PrimePart,
            2,
            false,
        )?,
        entry("lith-a1", "Lith A1 Relic", Category::Relic, 7, false)?,
        entry("rhino", "Rhino", Category::Frame, 1, true)?,
        entry("braton", "Braton", Category::Weapon, 3, true)?,
        entry(
            "lex-prime-receiver",
            "Lex Prime Receiver",
            Category::PrimePart,
            1,
            false,
        )?,
    ])?;
    let rewards = vec![
        RewardCandidate::new("Forma Blueprint", 12, 25, 0, false, 1.0)?,
        RewardCandidate::new("Lex Prime Receiver", 8, 15, 0, true, 1.0)?,
        RewardCandidate::new("Rare Prime Set", 30, 100, 0, false, 0.79)?,
        RewardCandidate::new("Paris Prime String", 6, 45, 1, false, 1.0)?,
    ];
    Ok(FakeSession {
        snapshot,
        meta: SnapshotMeta::fake("fake-build")?,
        rewards,
    })
}

fn entry(
    id: &str,
    name: &str,
    category: Category,
    quantity: u32,
    mastered: bool,
) -> Result<InventoryEntry, AppError> {
    let item = CatalogItem::new(ItemId::new(id)?, name, category)?;
    Ok(InventoryEntry::new(item, quantity).with_mastered(mastered))
}
