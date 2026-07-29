use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use async_trait::async_trait;
use fontferry_core::{
    Activity, FontFerryError, InstalledFont, ObservedFont, Result, StateRepository,
};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug)]
pub struct SqliteState {
    connection: Mutex<Connection>,
}

impl SqliteState {
    pub fn open(path: &Path) -> Result<Self> {
        let connection =
            Connection::open(path).map_err(|error| FontFerryError::State(error.to_string()))?;
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
        let json = serde_json::to_string(observed)
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        self.lock()?
            .execute(
                "INSERT INTO observed_fonts(font_id, record_json) VALUES (?1, ?2)
                 ON CONFLICT(font_id) DO UPDATE SET record_json = excluded.record_json",
                params![observed.font_id, json],
            )
            .map_err(|error| FontFerryError::State(error.to_string()))?;
        Ok(())
    }

    pub fn save_scan_result(&self, mut observed: ObservedFont) -> Result<()> {
        if let Some(existing) = self.get_observed_sync(&observed.font_id)? {
            observed.manual_version = existing.manual_version;
        }
        self.save_observed(&observed)
    }

    pub fn set_observed_manual_version(
        &self,
        font_id: &str,
        version: Option<String>,
    ) -> Result<()> {
        let mut observed = self
            .get_observed_sync(font_id)?
            .unwrap_or_else(|| ObservedFont {
                font_id: font_id.to_owned(),
                detected_version: None,
                manual_version: None,
                observed_files: Vec::new(),
                scanned_at: time::OffsetDateTime::now_utc(),
            });
        observed.manual_version = version;
        self.save_observed(&observed)
    }

    fn get_observed_sync(&self, font_id: &str) -> Result<Option<ObservedFont>> {
        let connection = self.lock()?;
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
