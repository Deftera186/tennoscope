use thiserror::Error;

use crate::catalog::ItemId;

#[derive(Debug, Error, PartialEq)]
pub enum DomainError {
    #[error("item ID must not be blank")]
    InvalidItemId,
    #[error("name must not be blank")]
    InvalidName,
    #[error("confidence must be finite and between 0.0 and 1.0")]
    InvalidConfidence,
    #[error("snapshot contains duplicate item ID: {0}")]
    DuplicateItemId(ItemId),
}
