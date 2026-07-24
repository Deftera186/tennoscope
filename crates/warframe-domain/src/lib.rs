#![forbid(unsafe_code)]

mod catalog;
mod inventory;
mod rewards;

pub use catalog::{CatalogItem, Category, DomainError, ItemId};
pub use inventory::{Collection, InventoryEntry, InventorySnapshot};
pub use rewards::{RewardAdvisor, RewardCandidate, RewardView};
