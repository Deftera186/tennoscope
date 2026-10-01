use local_store::SnapshotMeta;
use warframe_domain::{
    CatalogItem, Category, InventoryEntry, InventorySnapshot, ItemId, MasteryMark, RewardCandidate,
    SetPart,
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
        RewardCandidate::new("Forma Blueprint", 12, 25, 0, None, 1.0)?,
        RewardCandidate::new(
            "Lex Prime Receiver",
            8,
            15,
            0,
            Some(MasteryMark::Unmastered {
                subject: None,
                parts: vec![
                    SetPart {
                        name: "Blueprint".into(),
                        image: Some("blueprint.png".into()),
                        uses: 1,
                        held: 1,
                        this: false,
                    },
                    SetPart {
                        name: "Receiver".into(),
                        image: Some("GenericGunPrimeReceiver.png".into()),
                        uses: 1,
                        held: 0,
                        this: true,
                    },
                    SetPart {
                        name: "Barrel".into(),
                        image: Some("GenericGunPrimeBarrel.png".into()),
                        uses: 1,
                        held: 1,
                        this: false,
                    },
                ],
                missing: true,
                completes: true,
            }),
            1.0,
        )?,
        RewardCandidate::new("Rare Prime Set", 30, 100, 0, None, 0.79)?,
        RewardCandidate::new("Paris Prime String", 6, 45, 1, None, 1.0)?,
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
