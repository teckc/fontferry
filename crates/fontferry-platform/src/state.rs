use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use async_trait::async_trait;
use fontferry_core::{
    Activity, FontFerryError, InstalledFont, ObservedFont, Result, StateRepository,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

#[derive(Debug)]
pub struct SqliteState {
    connection: Mutex<Connection>,
}

impl SqliteState {
    pub fn open(path: &Path) -> Result<Self> {
        let journal = path.with_file_name("font-operation.json");
        let pending = journal
            .try_exists()
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        // A missing ledger is not an empty committed ledger. Never create one
        // beside recovery evidence. Missing ownership is never proof of a commit.
        let flags = if pending {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        };
        let connection = Connection::open_with_flags(path, flags).map_err(|error| {
            FontFerryError::State(format!(
                "cannot open font ledger; preserve database, journal and backups: {error}"
            ))
        })?;
        if pending {
            let has_ledger: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='installed_fonts')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| FontFerryError::State(error.to_string()))?;
            if !has_ledger {
                return Err(FontFerryError::State(
                    "recovery journal exists without an initialized font ledger; preserve all recovery data for repair".into(),
                ));
            }
            // Verify the columns recovery actually reads before any schema writes.
            // This is not an identity check for the ledger or its ownership records.
            connection
                .prepare("SELECT font_id, record_json FROM installed_fonts LIMIT 0")
                .map_err(|error| {
                    FontFerryError::State(format!(
                        "recovery ledger is not readable; preserve all recovery data: {error}"
                    ))
                })?;
        }
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        connection
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS schema_migrations (
                  version INTEGER PRIMARY KEY
                );
                CREATE TABLE IF NOT EXISTS operation_receipts (
                  operation_id TEXT PRIMARY KEY,
                  journal_digest TEXT NOT NULL,
                  font_id TEXT NOT NULL,
                  after_json TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS installed_fonts (
                  font_id TEXT PRIMARY KEY,
                  record_json TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS license_acceptance (
                  font_id TEXT NOT NULL,
                  revision TEXT NOT NULL,
                  accepted_at TEXT NOT NULL,
                  PRIMARY KEY (font_id, revision)
                );
                CREATE TABLE IF NOT EXISTS activity (
                  id TEXT PRIMARY KEY,
                  font_id TEXT,
                  level TEXT NOT NULL,
                  message TEXT NOT NULL,
                  created_at TEXT NOT NULL,
                  activity_json TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS settings (
                  key TEXT PRIMARY KEY,
                  value_json TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS user_sources (
                  font_id TEXT PRIMARY KEY,
                  definition_json TEXT NOT NULL,
                  updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS catalog_cache (
                  cache_key TEXT PRIMARY KEY,
                  body BLOB NOT NULL,
                  signature TEXT NOT NULL,
                  verified_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS observed_fonts (
                  font_id TEXT PRIMARY KEY,
                  record_json TEXT NOT NULL
                );
                INSERT OR IGNORE INTO schema_migrations(version) VALUES (1);
                "#,
            )
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn list_activity(&self, limit: usize) -> Result<Vec<Activity>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT activity_json FROM activity ORDER BY created_at DESC LIMIT ?1")
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        let rows = statement
            .query_map([limit.min(500) as i64], |row| row.get::<_, String>(0))
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        rows.map(|row| {
            let json = row.map_err(|error| FontFerryError::State(error.to_string()))?;
            serde_json::from_str(&json).map_err(|error| FontFerryError::State(error.to_string()))
        })
        .collect()
    }

    pub fn get_setting<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let connection = self.lock()?;
        let json: Option<String> = connection
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        json.map(|value| {
            serde_json::from_str(&value).map_err(|error| FontFerryError::State(error.to_string()))
        })
        .transpose()
    }

    pub fn set_setting<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let json = serde_json::to_string(value)
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        self.lock()?
            .execute(
                "INSERT INTO settings(key, value_json) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
                params![key, json],
            )
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        Ok(())
    }

    pub fn save_user_source(&self, definition: &fontferry_core::FontDefinition) -> Result<()> {
        definition.validate()?;
        let json = serde_json::to_string(definition)
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        self.lock()?
            .execute(
                "INSERT INTO user_sources(font_id, definition_json, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(font_id) DO UPDATE SET
                   definition_json = excluded.definition_json,
                   updated_at = excluded.updated_at",
                params![
                    definition.id,
                    json,
                    time::OffsetDateTime::now_utc().to_string()
                ],
            )
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        Ok(())
    }

    pub fn list_user_sources(&self) -> Result<Vec<fontferry_core::FontDefinition>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT definition_json FROM user_sources ORDER BY font_id")
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        rows.map(|row| {
            let json = row.map_err(|error| FontFerryError::State(error.to_string()))?;
            serde_json::from_str(&json).map_err(|error| FontFerryError::State(error.to_string()))
        })
        .collect()
    }

    pub fn save_observed(&self, observed: &ObservedFont) -> Result<()> {
        let connection = self.lock()?;
        Self::write_observed(&connection, observed)
    }

    fn write_observed(connection: &Connection, observed: &ObservedFont) -> Result<()> {
        let json = serde_json::to_string(observed)
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        connection
            .execute(
                "INSERT INTO observed_fonts(font_id, record_json) VALUES (?1, ?2)
                 ON CONFLICT(font_id) DO UPDATE SET record_json = excluded.record_json",
                params![observed.font_id, json],
            )
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        Ok(())
    }

    pub fn save_scan_result(&self, mut observed: ObservedFont) -> Result<()> {
        let mut connection = self.lock()?;
        // Both a running GUI and the scheduled process may have their own
        // connection. Start the write transaction before the read/merge.
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        if let Some(existing) = Self::read_observed(&transaction, &observed.font_id)? {
            if existing.scanned_at > observed.scanned_at {
                return Ok(());
            }
            observed.manual_version = existing.manual_version;
        }
        Self::write_observed(&transaction, &observed)?;
        transaction
            .commit()
            .map_err(|error| FontFerryError::State(error.to_string()))
    }

    pub fn set_observed_manual_version(
        &self,
        font_id: &str,
        version: Option<String>,
    ) -> Result<()> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        let mut observed =
            Self::read_observed(&transaction, font_id)?.unwrap_or_else(|| ObservedFont {
                font_id: font_id.to_owned(),
                detected_version: None,
                manual_version: None,
                observed_files: Vec::new(),
                // This row starts as a manual override, not a new scan.
                scanned_at: time::OffsetDateTime::UNIX_EPOCH,
            });
        observed.manual_version = version;
        Self::write_observed(&transaction, &observed)?;
        transaction
            .commit()
            .map_err(|error| FontFerryError::State(error.to_string()))
    }

    fn get_observed_sync(&self, font_id: &str) -> Result<Option<ObservedFont>> {
        let connection = self.lock()?;
        Self::read_observed(&connection, font_id)
    }

    fn read_observed(connection: &Connection, font_id: &str) -> Result<Option<ObservedFont>> {
        let json: Option<String> = connection
            .query_row(
                "SELECT record_json FROM observed_fonts WHERE font_id = ?1",
                [font_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        json.map(|value| {
            serde_json::from_str(&value).map_err(|error| FontFerryError::State(error.to_string()))
        })
        .transpose()
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| FontFerryError::State("database mutex is poisoned".into()))
    }
}

#[async_trait]
impl StateRepository for SqliteState {
    async fn commit_operation(
        &self,
        operation_id: &str,
        journal_digest: &str,
        font_id: &str,
        before: Option<&InstalledFont>,
        after: Option<&InstalledFont>,
    ) -> Result<()> {
        let error = |e: rusqlite::Error| FontFerryError::State(e.to_string());
        if operation_id.is_empty()
            || after.is_some_and(|r| r.font_id != font_id)
            || before.is_some_and(|r| r.font_id != font_id)
        {
            return Err(FontFerryError::State(
                "invalid operation receipt identity".into(),
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        let current: Option<String> = transaction
            .query_row(
                "SELECT record_json FROM installed_fonts WHERE font_id = ?1",
                [font_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(error)?;
        let current: Option<InstalledFont> = current
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| FontFerryError::State(e.to_string()))?;
        if current.as_ref() != before {
            return Err(FontFerryError::State(
                "ownership changed before commit; preserve recovery data".into(),
            ));
        }
        let after_json =
            serde_json::to_string(&after).map_err(|e| FontFerryError::State(e.to_string()))?;
        match after {
            Some(_) => {
                transaction.execute("INSERT INTO installed_fonts(font_id, record_json) VALUES (?1, ?2) ON CONFLICT(font_id) DO UPDATE SET record_json=excluded.record_json", params![font_id, after_json]).map_err(error)?;
            }
            None => {
                transaction
                    .execute("DELETE FROM installed_fonts WHERE font_id = ?1", [font_id])
                    .map_err(error)?;
            }
        }
        transaction.execute("INSERT INTO operation_receipts(operation_id, journal_digest, font_id, after_json) VALUES (?1, ?2, ?3, ?4)", params![operation_id, journal_digest, font_id, after_json]).map_err(error)?;
        transaction.commit().map_err(error)
    }

    async fn has_operation_receipt(
        &self,
        operation_id: &str,
        journal_digest: &str,
        font_id: &str,
        after: Option<&InstalledFont>,
    ) -> Result<bool> {
        let connection = self.lock()?;
        let row: Option<(String, String, String)> = connection.query_row("SELECT journal_digest, font_id, after_json FROM operation_receipts WHERE operation_id = ?1", [operation_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(|e| FontFerryError::State(e.to_string()))?;
        let Some((digest, id, json)) = row else {
            return Ok(false);
        };
        let recorded: Option<InstalledFont> =
            serde_json::from_str(&json).map_err(|e| FontFerryError::State(e.to_string()))?;
        Ok(digest == journal_digest && id == font_id && recorded.as_ref() == after)
    }

    async fn list_installed(&self) -> Result<Vec<InstalledFont>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT record_json FROM installed_fonts ORDER BY font_id")
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        rows.map(|row| {
            let json = row.map_err(|error| FontFerryError::State(error.to_string()))?;
            serde_json::from_str(&json).map_err(|error| FontFerryError::State(error.to_string()))
        })
        .collect()
    }

    async fn get_installed(&self, font_id: &str) -> Result<Option<InstalledFont>> {
        let connection = self.lock()?;
        let json: Option<String> = connection
            .query_row(
                "SELECT record_json FROM installed_fonts WHERE font_id = ?1",
                [font_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        json.map(|value| {
            serde_json::from_str(&value).map_err(|error| FontFerryError::State(error.to_string()))
        })
        .transpose()
    }

    async fn save_installed(&self, installed: &InstalledFont) -> Result<()> {
        let json = serde_json::to_string(installed)
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        self.lock()?
            .execute(
                "INSERT INTO installed_fonts(font_id, record_json) VALUES (?1, ?2)
                 ON CONFLICT(font_id) DO UPDATE SET record_json = excluded.record_json",
                params![installed.font_id, json],
            )
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        Ok(())
    }

    async fn remove_installed(&self, font_id: &str) -> Result<()> {
        self.lock()?
            .execute("DELETE FROM installed_fonts WHERE font_id = ?1", [font_id])
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        Ok(())
    }

    async fn is_license_accepted(&self, font_id: &str, revision: &str) -> Result<bool> {
        let count: i64 = self
            .lock()?
            .query_row(
                "SELECT COUNT(*) FROM license_acceptance
                 WHERE font_id = ?1 AND revision = ?2",
                params![font_id, revision],
                |row| row.get(0),
            )
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        Ok(count > 0)
    }

    async fn accept_license(&self, font_id: &str, revision: &str) -> Result<()> {
        self.lock()?
            .execute(
                "INSERT OR REPLACE INTO license_acceptance(font_id, revision, accepted_at)
                 VALUES (?1, ?2, ?3)",
                params![
                    font_id,
                    revision,
                    time::OffsetDateTime::now_utc().to_string()
                ],
            )
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        Ok(())
    }

    async fn append_activity(&self, activity: &Activity) -> Result<()> {
        let json = serde_json::to_string(activity)
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        self.lock()?
            .execute(
                "INSERT INTO activity(id, font_id, level, message, created_at, activity_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    activity.id,
                    activity.font_id,
                    format!("{:?}", activity.level).to_ascii_lowercase(),
                    activity.message,
                    activity.created_at.to_string(),
                    json
                ],
            )
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        Ok(())
    }

    async fn get_observed(&self, font_id: &str) -> Result<Option<ObservedFont>> {
        self.get_observed_sync(font_id)
    }
}

#[cfg(test)]
mod recovery_open_tests {
    use super::*;
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn background_scan_and_manual_override_preserve_each_others_fields() -> TestResult {
        let root = tempfile::tempdir()?;
        let path = root.path().join("fontferry.db");
        let scanner = SqliteState::open(&path)?;
        let manual = SqliteState::open(&path)?;
        let id = "font-awesome-pro";
        manual.set_observed_manual_version(id, Some("7.2".into()))?;
        let older = ObservedFont {
            font_id: id.into(),
            detected_version: Some("6.6".into()),
            manual_version: None,
            observed_files: vec!["old.otf".into()],
            scanned_at: time::OffsetDateTime::from_unix_timestamp(100)?,
        };
        let mut newer = older.clone();
        newer.detected_version = Some("7.1".into());
        newer.observed_files = vec!["new.otf".into()];
        newer.scanned_at = time::OffsetDateTime::from_unix_timestamp(200)?;
        scanner.save_scan_result(newer)?;
        scanner.save_scan_result(older)?;
        let observed = manual.get_observed_sync(id)?.ok_or("missing observation")?;
        assert_eq!(observed.manual_version.as_deref(), Some("7.2"));
        assert_eq!(observed.detected_version.as_deref(), Some("7.1"));
        manual.set_observed_manual_version(id, None)?;
        let observed = scanner
            .get_observed_sync(id)?
            .ok_or("missing observation")?;
        assert!(observed.manual_version.is_none());
        assert_eq!(observed.detected_version.as_deref(), Some("7.1"));
        Ok(())
    }

    #[test]
    fn pending_journal_does_not_create_missing_database() -> TestResult {
        let root = tempfile::tempdir()?;
        let journal = root.path().join("font-operation.json");
        std::fs::write(&journal, b"recovery evidence")?;
        let database = root.path().join("fontferry.db");
        assert!(SqliteState::open(&database).is_err());
        assert!(!database.exists());
        assert_eq!(std::fs::read(journal)?, b"recovery evidence");
        Ok(())
    }

    #[test]
    fn pending_journal_rejects_empty_database_without_initializing_schema() -> TestResult {
        let root = tempfile::tempdir()?;
        let database = root.path().join("fontferry.db");
        drop(Connection::open(&database)?);
        std::fs::write(root.path().join("font-operation.json"), b"evidence")?;
        assert!(SqliteState::open(&database).is_err());
        let connection = Connection::open(&database)?;
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 0);
        Ok(())
    }

    #[tokio::test]
    async fn legacy_database_without_pending_journal_migrates_without_losing_ownership()
    -> TestResult {
        let root = tempfile::tempdir()?;
        let database = root.path().join("fontferry.db");
        let connection = Connection::open(&database)?;
        connection.execute_batch(
            "CREATE TABLE installed_fonts (font_id TEXT PRIMARY KEY, record_json TEXT NOT NULL);",
        )?;
        let installed = InstalledFont {
            font_id: "legacy-font".into(),
            version: "1".into(),
            variant_ids: vec![],
            installed_at: time::OffsetDateTime::UNIX_EPOCH,
            owned_files: vec![],
            previous: None,
            manual_version: None,
        };
        connection.execute(
            "INSERT INTO installed_fonts VALUES (?1, ?2)",
            params![installed.font_id, serde_json::to_string(&installed)?],
        )?;
        drop(connection);
        let state = SqliteState::open(&database)?;
        assert_eq!(
            state.get_installed(&installed.font_id).await?,
            Some(installed)
        );
        assert!(
            !state
                .has_operation_receipt("absent", "digest", "legacy-font", None)
                .await?
        );
        Ok(())
    }

    #[test]
    fn initialized_ledger_can_reopen_for_pending_recovery() -> TestResult {
        let root = tempfile::tempdir()?;
        let database = root.path().join("fontferry.db");
        let state = SqliteState::open(&database)?;
        state.set_setting("update-channel", &"beta")?;
        drop(state);
        std::fs::write(root.path().join("font-operation.json"), b"evidence")?;
        let state = SqliteState::open(&database)?;
        assert_eq!(
            state.get_setting::<String>("update-channel")?,
            Some("beta".into())
        );
        Ok(())
    }

    #[test]
    fn corrupt_database_preserves_recovery_evidence() -> TestResult {
        let root = tempfile::tempdir()?;
        let database = root.path().join("fontferry.db");
        let journal = root.path().join("font-operation.json");
        let backup = root.path().join("backups");
        std::fs::create_dir(&backup)?;
        std::fs::write(backup.join("font.ttf"), b"saved font")?;
        std::fs::write(&database, b"not a SQLite database")?;
        std::fs::write(&journal, b"recovery evidence")?;
        assert!(SqliteState::open(&database).is_err());
        assert_eq!(std::fs::read(database)?, b"not a SQLite database");
        assert_eq!(std::fs::read(journal)?, b"recovery evidence");
        assert_eq!(std::fs::read(backup.join("font.ttf"))?, b"saved font");
        Ok(())
    }

    #[test]
    fn wrong_ledger_columns_rejected_before_schema_initialization() -> TestResult {
        let root = tempfile::tempdir()?;
        let database = root.path().join("fontferry.db");
        let connection = Connection::open(&database)?;
        connection.execute_batch("CREATE TABLE installed_fonts (unrelated TEXT);")?;
        std::fs::write(root.path().join("font-operation.json"), b"evidence")?;
        assert!(SqliteState::open(&database).is_err());
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn pending_recovery_reads_schema_and_ownership_from_live_wal() -> TestResult {
        let root = tempfile::tempdir()?;
        let database = root.path().join("fontferry.db");
        let writer = Connection::open(&database)?;
        writer.pragma_update(None, "journal_mode", "WAL")?;
        writer.pragma_update(None, "wal_autocheckpoint", 0)?;
        writer.execute_batch(
            "CREATE TABLE installed_fonts (font_id TEXT PRIMARY KEY, record_json TEXT NOT NULL);
             INSERT INTO installed_fonts VALUES ('test-font', 'retained ownership');",
        )?;
        let wal = root.path().join("fontferry.db-wal");
        assert!(std::fs::metadata(wal)?.len() > 0);
        std::fs::write(root.path().join("font-operation.json"), b"evidence")?;
        // Keep the writer alive: dropping it could checkpoint and hide a WAL read bug.
        let state = SqliteState::open(&database)?;
        let retained: String = state.lock()?.query_row(
            "SELECT record_json FROM installed_fonts WHERE font_id='test-font'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(retained, "retained ownership");
        drop(state);
        drop(writer);
        Ok(())
    }
}
