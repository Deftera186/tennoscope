#![forbid(unsafe_code)]

use std::path::Path;

use rusqlite::{Connection, params};
use thiserror::Error;
use warframe_domain::{
    CatalogItem, Category, Collection, DomainError, InventoryEntry, InventorySnapshot, ItemId,
};

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("invalid domain data: {0}")]
    Domain(#[from] DomainError),
    #[error("invalid category wire value: {0}")]
    Category(#[from] serde_json::Error),
    #[error("snapshot metadata fields must not be blank")]
    InvalidMetadata,
    #[error("database schema version {0} is not supported")]
    UnsupportedSchemaVersion(i64),
    #[error("stored integer is outside the domain range: {0}")]
    IntegerOutOfRange(i64),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotMeta {
    observed_at: String,
    game_build: String,
    source: String,
}

impl SnapshotMeta {
    pub fn new(
        observed_at: String,
        game_build: String,
        source: String,
    ) -> Result<Self, StoreError> {
        if [&observed_at, &game_build, &source]
            .into_iter()
            .any(|field| field.trim().is_empty())
        {
            return Err(StoreError::InvalidMetadata);
        }
        Ok(Self {
            observed_at,
            game_build,
            source,
        })
    }

    pub fn fake(build: impl Into<String>) -> Result<Self, StoreError> {
        Self::new(
            "2000-01-01T00:00:00Z".to_owned(),
            build.into(),
            "test-fixture".to_owned(),
        )
    }
}

pub struct SqliteStore {
    connection: Connection,
}

impl SqliteStore {
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::from_connection(Connection::open(path)?)
    }

    fn from_connection(connection: Connection) -> Result<Self, StoreError> {
        connection.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => connection.execute_batch(include_str!("schema.sql"))?,
            SCHEMA_VERSION => {}
            other => return Err(StoreError::UnsupportedSchemaVersion(other)),
        }
        Ok(Self { connection })
    }

    pub fn replace_collection(
        &mut self,
        snapshot: &InventorySnapshot,
        meta: &SnapshotMeta,
    ) -> Result<(), StoreError> {
        self.replace_collection_with_hook(snapshot, meta, || Ok(()))
    }

    fn replace_collection_with_hook<F>(
        &mut self,
        snapshot: &InventorySnapshot,
        meta: &SnapshotMeta,
        after_delete: F,
    ) -> Result<(), StoreError>
    where
        F: FnOnce() -> Result<(), StoreError>,
    {
        let transaction = self.connection.transaction()?;
        transaction.execute("DELETE FROM inventory", [])?;
        after_delete()?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO inventory (item_id, name, category, quantity, mastered) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for entry in snapshot.entries() {
                let category = serde_json::to_string(&entry.item.category)?;
                let category = category.trim_matches('"');
                statement.execute(params![
                    entry.item.id.as_str(),
                    entry.item.name,
                    category,
                    i64::from(entry.quantity),
                    entry.mastered,
                ])?;
            }
        }
        let item_count = i64::try_from(snapshot.entries().len())
            .map_err(|_| StoreError::IntegerOutOfRange(i64::MAX))?;
        transaction.execute(
            "INSERT INTO snapshot_audit (observed_at, game_build, source, item_count) \
             VALUES (?1, ?2, ?3, ?4)",
            params![meta.observed_at, meta.game_build, meta.source, item_count],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn load_collection(&self) -> Result<Collection, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT item_id, name, category, quantity, mastered FROM inventory ORDER BY item_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })?;
        let raw_entries = rows.collect::<Result<Vec<_>, _>>()?;
        let entries = raw_entries
            .into_iter()
            .map(|(id, name, category, quantity, mastered)| {
                let category = serde_json::from_str::<Category>(&format!("\"{category}\""))?;
                let quantity =
                    u32::try_from(quantity).map_err(|_| StoreError::IntegerOutOfRange(quantity))?;
                let item = CatalogItem::new(ItemId::new(id)?, name, category)?;
                Ok(InventoryEntry::new(item, quantity).with_mastered(mastered))
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let snapshot = InventorySnapshot::coherent(entries)?;
        let mut collection = Collection::default();
        collection.replace(snapshot);
        Ok(collection)
    }

    pub fn audit_count(&self) -> Result<u64, StoreError> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM snapshot_audit", [], |row| row.get(0))?;
        u64::try_from(count).map_err(|_| StoreError::IntegerOutOfRange(count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(quantity: u32) -> InventorySnapshot {
        let item = CatalogItem::new(ItemId::new("lex").unwrap(), "Lex", Category::Weapon).unwrap();
        InventorySnapshot::coherent(vec![InventoryEntry::new(item, quantity)]).unwrap()
    }

    #[test]
    fn failed_insert_rolls_back_delete_and_audit() {
        let mut store = SqliteStore::in_memory().unwrap();
        store
            .replace_collection(&snapshot(3), &SnapshotMeta::fake("before").unwrap())
            .unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_inventory BEFORE INSERT ON inventory \
                 BEGIN SELECT RAISE(ABORT, 'test failure'); END;",
            )
            .unwrap();

        assert!(
            store
                .replace_collection(&snapshot(1), &SnapshotMeta::fake("after").unwrap())
                .is_err()
        );
        assert_eq!(
            store
                .load_collection()
                .unwrap()
                .quantity(&ItemId::new("lex").unwrap()),
            3
        );
        assert_eq!(store.audit_count().unwrap(), 1);
    }

    #[test]
    fn failure_immediately_after_delete_rolls_back_snapshot_and_audit() {
        let mut store = SqliteStore::in_memory().unwrap();
        store
            .replace_collection(&snapshot(3), &SnapshotMeta::fake("before").unwrap())
            .unwrap();

        assert!(
            store
                .replace_collection_with_hook(
                    &snapshot(1),
                    &SnapshotMeta::fake("after").unwrap(),
                    || Err(StoreError::InvalidMetadata),
                )
                .is_err()
        );
        assert_eq!(
            store
                .load_collection()
                .unwrap()
                .quantity(&ItemId::new("lex").unwrap()),
            3
        );
        assert_eq!(store.audit_count().unwrap(), 1);
    }

    #[test]
    fn corrupt_category_and_integer_rows_return_errors() {
        let store = SqliteStore::in_memory().unwrap();
        store
            .connection
            .execute(
                "INSERT INTO inventory VALUES ('bad', 'Bad', 'unknown', 1, 0)",
                [],
            )
            .unwrap();
        assert!(store.load_collection().is_err());

        store
            .connection
            .execute("DELETE FROM inventory", [])
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO inventory VALUES ('huge', 'Huge', 'weapon', ?1, 0)",
                [i64::MAX],
            )
            .unwrap();
        assert!(store.load_collection().is_err());
    }

    #[test]
    fn corrupt_blank_domain_fields_return_errors() {
        for (id, name) in [(" ", "Bad"), ("bad", "\t")] {
            let store = SqliteStore::in_memory().unwrap();
            store
                .connection
                .execute(
                    "INSERT INTO inventory VALUES (?1, ?2, 'weapon', 1, 0)",
                    params![id, name],
                )
                .unwrap();
            assert!(store.load_collection().is_err());
        }
    }
}
