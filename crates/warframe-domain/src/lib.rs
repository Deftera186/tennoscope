#![forbid(unsafe_code)]

mod catalog;
mod error;
mod inventory;
mod rewards;

pub use catalog::{CatalogItem, Category, ItemId};
pub use error::DomainError;
pub use inventory::{Collection, InventoryEntry, InventorySnapshot};
pub use rewards::{RewardAdvisor, RewardCandidate, RewardView};
