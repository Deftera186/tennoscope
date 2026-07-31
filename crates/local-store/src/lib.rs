#![forbid(unsafe_code)]

use std::path::Path;

use rusqlite::{Connection, params, types::Value};
use serde::Serialize;
use thiserror::Error;
use warframe_domain::{
    CatalogItem, Category, Collection, DomainError, InventoryEntry, InventorySnapshot, ItemId,
};

const SCHEMA_VERSION: i64 = 4;
const SCHEMA_SQL: &str = include_str!("schema.sql");

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
    #[error("invalid database schema: {0}")]
    Schema(String),
    #[error("corrupt inventory row {item_id:?}, field {field}: {detail}")]
    CorruptRow {
        item_id: String,
        field: &'static str,
        detail: String,
    },
    #[error("stored integer is outside the domain range: {0}")]
    IntegerOutOfRange(i64),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
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

    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn game_build(&self) -> &str {
        &self.game_build
    }

    pub fn is_fake(&self) -> bool {
        self.source == "test-fixture"
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

    fn from_connection(mut connection: Connection) -> Result<Self, StoreError> {
        connection.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => initialize_schema(&mut connection)?,
            1 => {
                migrate_v1_to_v2(&mut connection)?;
                migrate_v2_to_v3(&mut connection)?;
                migrate_v3_to_v4(&mut connection)?;
            }
            2 => {
                migrate_v2_to_v3(&mut connection)?;
                migrate_v3_to_v4(&mut connection)?;
            }
            3 => migrate_v3_to_v4(&mut connection)?,
            SCHEMA_VERSION => validate_schema(&connection)?,
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
                "INSERT INTO inventory \
                 (item_id, name, category, quantity, mastered, image_name, rank, max_rank) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for entry in snapshot.entries() {
                let category = encode_category(entry.item.category)?;
                statement.execute(params![
                    entry.item.id.as_str(),
                    entry.item.name,
                    category,
                    i64::from(entry.quantity),
                    entry.mastered,
                    entry.item.image_name,
                    entry.rank,
                    entry.max_rank,
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
            "SELECT item_id, name, category, quantity, mastered, image_name, rank, max_rank \
             FROM inventory ORDER BY item_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, Value>(0)?,
                row.get::<_, Value>(1)?,
                row.get::<_, Value>(2)?,
                row.get::<_, Value>(3)?,
                row.get::<_, Value>(4)?,
                row.get::<_, Value>(5)?,
                row.get::<_, Value>(6)?,
                row.get::<_, Value>(7)?,
            ))
        })?;
        let raw_entries = rows.collect::<Result<Vec<_>, _>>()?;
        let entries = raw_entries
            .into_iter()
            .map(
                |(id, name, category, quantity, mastered, image_name, rank, max_rank)| {
                    let id = expect_text(id, "<unknown>", "item_id")?;
                    let name = expect_text(name, &id, "name")?;
                    let category = expect_text(category, &id, "category")?;
                    let quantity = expect_integer(quantity, &id, "quantity")?;
                    let mastered = expect_integer(mastered, &id, "mastered")?;
                    let image_name = expect_optional_text(image_name, &id, "image_name")?;
                    let rank = expect_optional_rank(rank, &id, "rank")?;
                    let max_rank = expect_optional_rank(max_rank, &id, "max_rank")?;
                    let category = decode_category(&id, category)?;
                    let quantity = u32::try_from(quantity)
                        .map_err(|error| corrupt_row(&id, "quantity", error))?;
                    let mastered = match mastered {
                        0 => false,
                        1 => true,
                        value => {
                            return Err(corrupt_row(
                                &id,
                                "mastered",
                                format!("expected 0 or 1, found {value}"),
                            ));
                        }
                    };
                    let item_id = ItemId::new(id.clone())
                        .map_err(|error| corrupt_row(&id, "item_id", error))?;
                    let item = CatalogItem::new(item_id, name, category)
                        .map_err(|error| corrupt_row(&id, "name", error))?;
                    let item = match image_name {
                        Some(image_name) => item
                            .with_image_name(image_name)
                            .map_err(|error| corrupt_row(&id, "image_name", error))?,
                        None => item,
                    };
                    let entry = InventoryEntry::new(item, quantity).with_mastered(mastered);
                    Ok(match rank {
                        Some(rank) => entry.with_rank(rank, max_rank),
                        None => entry,
                    })
                },
            )
            .collect::<Result<Vec<_>, StoreError>>()?;
        let snapshot = InventorySnapshot::coherent(entries)?;
        let mut collection = Collection::default();
        collection.replace(snapshot);
        Ok(collection)
    }

    /// Updates catalog-owned fields without changing quantities or snapshot freshness.
    pub fn update_collection_metadata(
        &mut self,
        snapshot: &InventorySnapshot,
    ) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        let mut statement = transaction.prepare(
            "UPDATE inventory SET name = ?2, category = ?3, image_name = ?4 WHERE item_id = ?1",
        )?;
        for entry in snapshot.entries() {
            statement.execute(params![
                entry.item.id.as_str(),
                entry.item.name,
                encode_category(entry.item.category)?,
                entry.item.image_name,
            ])?;
        }
        drop(statement);
        transaction.commit()?;
        Ok(())
    }

    pub fn audit_count(&self) -> Result<u64, StoreError> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM snapshot_audit", [], |row| row.get(0))?;
        u64::try_from(count).map_err(|_| StoreError::IntegerOutOfRange(count))
    }

    pub fn latest_snapshot_meta(&self) -> Result<Option<SnapshotMeta>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT observed_at, game_build, source FROM snapshot_audit ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = statement.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(SnapshotMeta::new(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
        )?))
    }

    /// The stored warframe.market token, if this installation fell back to the database.
    ///
    /// Returned as a string rather than a credential type: this crate is the bottom of the stack
    /// and has no business knowing what a market token is. The crate above wraps it.
    pub fn market_credential(&self) -> Result<Option<String>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT token FROM market_credential WHERE id = 1")?;
        let mut rows = statement.query([])?;
        rows.next()?.map(|row| row.get(0)).transpose().map_err(Into::into)
    }

    pub fn set_market_credential(&mut self, token: &str) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO market_credential (id, token) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET token = excluded.token",
            params![token],
        )?;
        Ok(())
    }

    pub fn clear_market_credential(&mut self) -> Result<(), StoreError> {
        self.connection
            .execute("DELETE FROM market_credential WHERE id = 1", [])?;
        Ok(())
    }
}

fn initialize_schema(connection: &mut Connection) -> Result<(), StoreError> {
    let conflicting_tables: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'table' AND name IN ('inventory', 'snapshot_audit')",
        [],
        |row| row.get(0),
    )?;
    if conflicting_tables != 0 {
        return Err(StoreError::Schema(
            "version 0 database contains conflicting inventory or snapshot_audit table".to_owned(),
        ));
    }

    let transaction = connection.transaction()?;
    transaction.execute_batch(SCHEMA_SQL)?;
    validate_schema(&transaction)?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

/// Adds `image_name`. Leaves the version at 2 rather than at `SCHEMA_VERSION`, because the v3
/// migration runs straight after it and is what finishes the job.
fn migrate_v1_to_v2(connection: &mut Connection) -> Result<(), StoreError> {
    let transaction = connection.transaction()?;
    transaction.execute("ALTER TABLE inventory ADD COLUMN image_name TEXT", [])?;
    transaction.pragma_update(None, "user_version", 2)?;
    transaction.commit()?;
    Ok(())
}

/// Adds the rank a mod or arcane was fused to.
///
/// Every existing row is unranked, which is both the honest answer for a database that never
/// recorded one and the state the next sync overwrites anyway.
fn migrate_v2_to_v3(connection: &mut Connection) -> Result<(), StoreError> {
    let transaction = connection.transaction()?;
    transaction.execute(
        "ALTER TABLE inventory ADD COLUMN rank INTEGER CHECK (rank IS NULL OR rank >= 0)",
        [],
    )?;
    transaction.execute(
        "ALTER TABLE inventory ADD COLUMN max_rank INTEGER CHECK (max_rank IS NULL OR max_rank >= 0)",
        [],
    )?;
    transaction.pragma_update(None, "user_version", 3)?;
    transaction.commit()?;
    Ok(())
}

/// Adds somewhere to keep the warframe.market token when no OS keyring is available.
///
/// Empty for every existing database, which is the honest state: nothing was linked before this
/// migration existed.
fn migrate_v3_to_v4(connection: &mut Connection) -> Result<(), StoreError> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE market_credential (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            token TEXT NOT NULL CHECK (length(trim(token)) > 0)
        );",
    )?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    validate_schema(&transaction)?;
    transaction.commit()?;
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
struct SchemaColumn {
    name: String,
    data_type: String,
    not_null: bool,
    primary_key_position: i64,
}

fn validate_schema(connection: &Connection) -> Result<(), StoreError> {
    validate_table(
        connection,
        "inventory",
        &[
            ("item_id", "TEXT", false, 1),
            ("name", "TEXT", true, 0),
            ("category", "TEXT", true, 0),
            ("quantity", "INTEGER", true, 0),
            ("mastered", "INTEGER", true, 0),
            ("image_name", "TEXT", false, 0),
            ("rank", "INTEGER", false, 0),
            ("max_rank", "INTEGER", false, 0),
        ],
    )?;
    validate_table(
        connection,
        "snapshot_audit",
        &[
            ("id", "INTEGER", false, 1),
            ("observed_at", "TEXT", true, 0),
            ("game_build", "TEXT", true, 0),
            ("source", "TEXT", true, 0),
            ("item_count", "INTEGER", true, 0),
        ],
    )?;
    validate_canonical_table(connection, "inventory")?;
    validate_canonical_table(connection, "snapshot_audit")?;
    validate_canonical_table(connection, "market_credential")?;
    validate_constraints(connection)
}

fn validate_canonical_table(
    connection: &Connection,
    table: &'static str,
) -> Result<(), StoreError> {
    let actual: Option<String> = connection.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    let expected = canonical_table_definition(table)?;
    if normalize_sql(actual.as_deref().unwrap_or_default()) != normalize_sql(expected) {
        return Err(StoreError::Schema(format!(
            "table {table} does not match the canonical schema constraint definition \
             (including NOT NULL, PK, CHECK, and AUTOINCREMENT requirements)"
        )));
    }
    Ok(())
}

fn canonical_table_definition(table: &'static str) -> Result<&'static str, StoreError> {
    let prefix = format!("createtable{table}(");
    SCHEMA_SQL
        .split(';')
        .find(|statement| normalize_sql(statement).starts_with(&prefix))
        .ok_or_else(|| {
            StoreError::Schema(format!(
                "canonical schema definition for table {table} is missing"
            ))
        })
}

fn normalize_sql(sql: &str) -> String {
    let mut normalized = String::with_capacity(sql.len());
    let mut characters = sql.chars().peekable();
    let mut in_string = false;
    while let Some(character) = characters.next() {
        if in_string {
            normalized.push(character);
            if character == '\'' {
                if characters.peek() == Some(&'\'') {
                    if let Some(escaped_quote) = characters.next() {
                        normalized.push(escaped_quote);
                    }
                } else {
                    in_string = false;
                }
            }
            continue;
        }

        match character {
            '\'' => {
                in_string = true;
                normalized.push(character);
            }
            '-' if characters.peek() == Some(&'-') => {
                characters.next();
                for comment_character in characters.by_ref() {
                    if comment_character == '\n' {
                        break;
                    }
                }
            }
            '"' | '`' | '[' | ']' | ';' => {}
            character if character.is_ascii_whitespace() => {}
            character => normalized.push(character.to_ascii_lowercase()),
        }
    }
    normalized
}

fn validate_constraints(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch("SAVEPOINT validate_schema_constraints")?;
    let validation = (|| {
        let probe_id = unused_probe_id(connection)?;
        require_rejected(
            connection,
            "inventory quantity >= 0 constraint",
            "INSERT INTO inventory (item_id, name, category, quantity, mastered) \
             VALUES (?1, 'Probe', 'weapon', -1, 0)",
            [&probe_id],
        )?;
        require_rejected(
            connection,
            "inventory mastered 0/1 constraint",
            "INSERT INTO inventory (item_id, name, category, quantity, mastered) \
             VALUES (?1, 'Probe', 'weapon', 0, 2)",
            [&probe_id],
        )?;
        for (label, observed_at, game_build, source, item_count) in [
            (
                "snapshot_audit observed_at nonblank constraint",
                " ",
                "build",
                "probe",
                0,
            ),
            (
                "snapshot_audit game_build nonblank constraint",
                "now",
                " ",
                "probe",
                0,
            ),
            (
                "snapshot_audit source nonblank constraint",
                "now",
                "build",
                " ",
                0,
            ),
            (
                "snapshot_audit item_count >= 0 constraint",
                "now",
                "build",
                "probe",
                -1,
            ),
        ] {
            require_rejected(
                connection,
                label,
                "INSERT INTO snapshot_audit (observed_at, game_build, source, item_count) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![observed_at, game_build, source, item_count],
            )?;
        }

        let insert_audit = "INSERT INTO snapshot_audit (observed_at, game_build, source, item_count) \
             VALUES ('now', 'build', 'schema-probe', 0)";
        connection.execute(insert_audit, [])?;
        let first_id = connection.last_insert_rowid();
        connection.execute("DELETE FROM snapshot_audit WHERE id = ?1", [first_id])?;
        connection.execute(insert_audit, [])?;
        let second_id = connection.last_insert_rowid();
        if second_id <= first_id {
            return Err(StoreError::Schema(
                "snapshot_audit id is missing AUTOINCREMENT semantics".to_owned(),
            ));
        }
        Ok(())
    })();
    let cleanup = connection.execute_batch(
        "ROLLBACK TO validate_schema_constraints; RELEASE validate_schema_constraints",
    );
    match (validation, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error.into()),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn unused_probe_id(connection: &Connection) -> Result<String, StoreError> {
    for suffix in 0_u16..1024 {
        let candidate = format!("__local_store_schema_probe_{suffix}__");
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM inventory WHERE item_id = ?1)",
            [&candidate],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(candidate);
        }
    }
    Err(StoreError::Schema(
        "could not allocate a temporary inventory ID for constraint validation".to_owned(),
    ))
}

fn require_rejected<P>(
    connection: &Connection,
    label: &str,
    sql: &str,
    params: P,
) -> Result<(), StoreError>
where
    P: rusqlite::Params,
{
    if connection.execute(sql, params).is_ok() {
        return Err(StoreError::Schema(format!(
            "required constraint is missing: {label}"
        )));
    }
    Ok(())
}

fn validate_table(
    connection: &Connection,
    table: &'static str,
    expected: &[(&str, &str, bool, i64)],
) -> Result<(), StoreError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let actual = statement
        .query_map([], |row| {
            Ok(SchemaColumn {
                name: row.get(1)?,
                data_type: row.get(2)?,
                not_null: row.get::<_, i64>(3)? != 0,
                primary_key_position: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let expected = expected
        .iter()
        .map(
            |(name, data_type, not_null, primary_key_position)| SchemaColumn {
                name: (*name).to_owned(),
                data_type: (*data_type).to_owned(),
                not_null: *not_null,
                primary_key_position: *primary_key_position,
            },
        )
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(StoreError::Schema(format!(
            "table {table} has incompatible columns: expected {expected:?}, found {actual:?}"
        )));
    }
    Ok(())
}

fn encode_category(category: Category) -> Result<String, StoreError> {
    match serde_json::to_value(category)? {
        serde_json::Value::String(wire) => Ok(wire),
        other => Err(StoreError::Schema(format!(
            "category serializer produced non-string value {other}"
        ))),
    }
}

fn decode_category(item_id: &str, wire: String) -> Result<Category, StoreError> {
    serde_json::from_value(serde_json::Value::String(wire))
        .map_err(|error| corrupt_row(item_id, "category", error))
}

fn expect_text(value: Value, item_id: &str, field: &'static str) -> Result<String, StoreError> {
    match value {
        Value::Text(value) => Ok(value),
        other => Err(corrupt_row(
            item_id,
            field,
            format!("expected TEXT, found {:?}", other.data_type()),
        )),
    }
}

/// A stored rank, or nothing for the unranked stack and for everything that cannot be ranked.
fn expect_optional_rank(
    value: Value,
    item_id: &str,
    field: &'static str,
) -> Result<Option<u32>, StoreError> {
    match value {
        Value::Null => Ok(None),
        Value::Integer(value) => u32::try_from(value)
            .map(Some)
            .map_err(|error| corrupt_row(item_id, field, error)),
        other => Err(corrupt_row(
            item_id,
            field,
            format!("expected INTEGER or NULL, found {:?}", other.data_type()),
        )),
    }
}

fn expect_optional_text(
    value: Value,
    item_id: &str,
    field: &'static str,
) -> Result<Option<String>, StoreError> {
    match value {
        Value::Null => Ok(None),
        Value::Text(value) => Ok(Some(value)),
        other => Err(corrupt_row(
            item_id,
            field,
            format!("expected TEXT or NULL, found {:?}", other.data_type()),
        )),
    }
}

fn expect_integer(value: Value, item_id: &str, field: &'static str) -> Result<i64, StoreError> {
    match value {
        Value::Integer(value) => Ok(value),
        other => Err(corrupt_row(
            item_id,
            field,
            format!("expected INTEGER, found {:?}", other.data_type()),
        )),
    }
}

fn corrupt_row(item_id: &str, field: &'static str, detail: impl std::fmt::Display) -> StoreError {
    StoreError::CorruptRow {
        item_id: item_id.to_owned(),
        field,
        detail: detail.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database_with(sql: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("malformed.sqlite3");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch(sql).unwrap();
        drop(connection);
        (directory, path)
    }

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
                "INSERT INTO inventory (item_id, name, category, quantity, mastered, image_name) VALUES ('bad', 'Bad', 'unknown', 1, 0, NULL)",
                [],
            )
            .unwrap();
        let error = store.load_collection().unwrap_err().to_string();
        assert!(error.contains("bad"));
        assert!(error.contains("category"));

        store
            .connection
            .execute("DELETE FROM inventory", [])
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO inventory (item_id, name, category, quantity, mastered, image_name) VALUES ('huge', 'Huge', 'weapon', ?1, 0, NULL)",
                [i64::MAX],
            )
            .unwrap();
        let error = store.load_collection().unwrap_err().to_string();
        assert!(error.contains("huge"));
        assert!(error.contains("quantity"));
    }

    #[test]
    fn corrupt_blank_domain_fields_return_errors() {
        for (id, name) in [(" ", "Bad"), ("bad", "\t")] {
            let store = SqliteStore::in_memory().unwrap();
            store
                .connection
                .execute(
                    "INSERT INTO inventory (item_id, name, category, quantity, mastered, image_name) VALUES (?1, ?2, 'weapon', 1, 0, NULL)",
                    params![id, name],
                )
                .unwrap();
            let error = store.load_collection().unwrap_err().to_string();
            assert!(error.contains("item_id") || error.contains("name"));
        }
    }

    #[test]
    fn corrupt_mastered_integer_has_row_and_field_context() {
        for value in [2, -1] {
            let store = SqliteStore::in_memory().unwrap();
            store
                .connection
                .pragma_update(None, "ignore_check_constraints", true)
                .unwrap();
            store
                .connection
                .execute(
                    "INSERT INTO inventory (item_id, name, category, quantity, mastered, image_name) VALUES ('lex', 'Lex', 'weapon', 1, ?1, NULL)",
                    [value],
                )
                .unwrap();

            let error = store.load_collection().unwrap_err().to_string();
            assert!(error.contains("lex"));
            assert!(error.contains("mastered"));
        }
    }

    #[test]
    fn corrupt_sqlite_value_types_have_row_and_field_context() {
        for (column, sql) in [
            (
                "quantity",
                "INSERT INTO inventory (item_id, name, category, quantity, mastered, image_name) VALUES ('lex', 'Lex', 'weapon', X'01', 0, NULL)",
            ),
            (
                "name",
                "INSERT INTO inventory (item_id, name, category, quantity, mastered, image_name) VALUES ('lex', X'01', 'weapon', 1, 0, NULL)",
            ),
        ] {
            let store = SqliteStore::in_memory().unwrap();
            store.connection.execute(sql, []).unwrap();

            let error = store.load_collection().unwrap_err().to_string();
            assert!(error.contains("lex"));
            assert!(error.contains(column));
        }
    }

    #[test]
    fn version_zero_database_with_conflicting_table_is_rejected() {
        let (_directory, path) = database_with("CREATE TABLE inventory (wrong TEXT);");

        let error = SqliteStore::open(&path).err().unwrap().to_string();
        assert!(error.contains("schema"));
        assert!(error.contains("inventory"));
    }

    #[test]
    fn version_one_database_missing_required_table_is_rejected_at_open() {
        let (_directory, path) = database_with(
            "CREATE TABLE inventory (
                item_id TEXT PRIMARY KEY, name TEXT NOT NULL, category TEXT NOT NULL,
                quantity INTEGER NOT NULL, mastered INTEGER NOT NULL
             );
             PRAGMA user_version = 1;",
        );

        let error = SqliteStore::open(&path).err().unwrap().to_string();
        assert!(error.contains("schema"));
        assert!(error.contains("snapshot_audit"));
    }

    #[test]
    fn version_one_database_with_incompatible_column_is_rejected_at_open() {
        let (_directory, path) = database_with(
            "CREATE TABLE inventory (
                item_id TEXT PRIMARY KEY, name TEXT, category TEXT NOT NULL,
                quantity INTEGER NOT NULL, mastered INTEGER NOT NULL
             );
             CREATE TABLE snapshot_audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT, observed_at TEXT NOT NULL,
                game_build TEXT NOT NULL, source TEXT NOT NULL, item_count INTEGER NOT NULL
             );
             PRAGMA user_version = 1;",
        );

        let error = SqliteStore::open(&path).err().unwrap().to_string();
        assert!(error.contains("schema"));
        assert!(error.contains("name"));
    }

    #[test]
    fn version_one_database_with_missing_checks_is_rejected_at_open() {
        let (_directory, path) = database_with(
            "CREATE TABLE inventory (
                item_id TEXT PRIMARY KEY, name TEXT NOT NULL, category TEXT NOT NULL,
                quantity INTEGER NOT NULL, mastered INTEGER NOT NULL
             );
             CREATE TABLE snapshot_audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT, observed_at TEXT NOT NULL,
                game_build TEXT NOT NULL, source TEXT NOT NULL, item_count INTEGER NOT NULL
             );
             PRAGMA user_version = 1;",
        );

        let error = SqliteStore::open(&path).err().unwrap().to_string();
        assert!(error.contains("schema"));
        assert!(error.contains("constraint"));
    }

    #[test]
    fn version_one_database_without_audit_autoincrement_is_rejected_at_open() {
        let (_directory, path) = database_with(
            "CREATE TABLE inventory (
                item_id TEXT PRIMARY KEY, name TEXT NOT NULL, category TEXT NOT NULL,
                quantity INTEGER NOT NULL CHECK (quantity >= 0),
                mastered INTEGER NOT NULL CHECK (mastered IN (0, 1))
             );
             CREATE TABLE snapshot_audit (
                id INTEGER PRIMARY KEY,
                observed_at TEXT NOT NULL CHECK (length(trim(observed_at)) > 0),
                game_build TEXT NOT NULL CHECK (length(trim(game_build)) > 0),
                source TEXT NOT NULL CHECK (length(trim(source)) > 0),
                item_count INTEGER NOT NULL CHECK (item_count >= 0)
             );
             PRAGMA user_version = 1;",
        );

        let error = SqliteStore::open(&path).err().unwrap().to_string();
        assert!(error.contains("schema"));
        assert!(error.contains("AUTOINCREMENT"));
    }

    #[test]
    fn version_one_database_with_probe_shaped_weaker_checks_is_rejected() {
        let (_directory, path) = database_with(
            "CREATE TABLE inventory (
                item_id TEXT PRIMARY KEY, name TEXT NOT NULL, category TEXT NOT NULL,
                quantity INTEGER NOT NULL CHECK (quantity != -1),
                mastered INTEGER NOT NULL CHECK (mastered != 2)
             );
             CREATE TABLE snapshot_audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                observed_at TEXT NOT NULL CHECK (observed_at != ' '),
                game_build TEXT NOT NULL CHECK (game_build != ' '),
                source TEXT NOT NULL CHECK (source != ' '),
                item_count INTEGER NOT NULL CHECK (item_count != -1)
             );
             PRAGMA user_version = 1;",
        );

        let error = SqliteStore::open(&path).err().unwrap().to_string();
        assert!(error.contains("schema"));
        assert!(error.contains("canonical"));
    }
}
