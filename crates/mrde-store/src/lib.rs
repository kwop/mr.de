use anyhow::Context;
use mrde_core::models::{MergeRequest, Review};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > 1 {
            anyhow::bail!("Database was created by a newer mr.de version");
        }
        if version == 0 {
            connection.execute_batch(include_str!("../migrations/001_initial.sql"))?;
        }
        Ok(Self { connection })
    }
    pub fn queue(&self, instance: &str) -> anyhow::Result<Vec<MergeRequest>> {
        let raw: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM queues WHERE instance=?1",
                [instance],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|s| serde_json::from_str(&s))
            .transpose()
            .map(|v| v.unwrap_or_default())
            .context("Invalid cached queue")
    }
    pub fn save_queue(&self, instance: &str, queue: &[MergeRequest]) -> anyhow::Result<()> {
        self.connection.execute("INSERT INTO queues(instance,payload) VALUES (?1,?2) ON CONFLICT(instance) DO UPDATE SET payload=excluded.payload", params![instance, serde_json::to_string(queue)?])?;
        Ok(())
    }
    pub fn review(&self, key: &str) -> anyhow::Result<Option<Review>> {
        let raw: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM reviews WHERE cache_key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|s| serde_json::from_str(&s))
            .transpose()
            .context("Invalid cached review")
    }
    pub fn save_review(&self, key: &str, review: &Review) -> anyhow::Result<()> {
        self.connection.execute("INSERT INTO reviews(cache_key,payload) VALUES (?1,?2) ON CONFLICT(cache_key) DO UPDATE SET payload=excluded.payload", params![key, serde_json::to_string(review)?])?;
        Ok(())
    }
    /// Any JSON result stored under a namespaced key (tours, analyses) in the review cache.
    pub fn cached<T: serde::de::DeserializeOwned>(&self, key: &str) -> anyhow::Result<Option<T>> {
        let raw: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM reviews WHERE cache_key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()?;
        // An entry written by an older format is a cache miss, not an error.
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
    }
    pub fn save_cached<T: serde::Serialize>(&self, key: &str, value: &T) -> anyhow::Result<()> {
        self.connection.execute(
            "INSERT INTO reviews(cache_key,payload) VALUES (?1,?2) ON CONFLICT(cache_key) DO UPDATE SET payload=excluded.payload",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }
    pub fn record(&self, action: &str, subject: &str) -> anyhow::Result<()> {
        self.connection.execute(
            "INSERT INTO history(action,subject) VALUES (?1,?2)",
            params![action, subject],
        )?;
        Ok(())
    }
}

/// Values must never be logged or serialized into application configuration.
pub fn read_secret(instance: &str, name: &str) -> anyhow::Result<String> {
    let entry = keyring::Entry::new("mrde", &format!("{instance}:{name}"))?;
    match entry.get_password() {
        Ok(value) => Ok(value),
        Err(keyring::Error::NoEntry) => Ok(String::new()),
        Err(error) => Err(error).context("Cannot read macOS Keychain"),
    }
}
pub fn write_secret(instance: &str, name: &str, value: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new("mrde", &format!("{instance}:{name}"))?;
    if value.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    } else {
        entry
            .set_password(value)
            .context("Cannot write macOS Keychain")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_is_repeatable_and_cache_is_isolated() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        store
            .connection
            .execute_batch(include_str!("../migrations/001_initial.sql"))
            .unwrap();
        let review = Review {
            summary: "Summary".into(),
            intention: "Intent".into(),
            risks: vec![],
            priority_files: vec![],
            confidence: 0.8,
        };
        store
            .save_review("instance/project/sha/base/prompt", &review)
            .unwrap();
        assert!(store.review("another/sha").unwrap().is_none());
        store.save_cached("tour:key", &vec![1, 2]).unwrap();
        assert_eq!(
            store.cached::<Vec<u8>>("tour:key").unwrap(),
            Some(vec![1, 2])
        );
        assert_eq!(
            store.cached::<String>("tour:key").unwrap(),
            None,
            "format change is a miss"
        );
        assert_eq!(
            store
                .review("instance/project/sha/base/prompt")
                .unwrap()
                .unwrap()
                .summary,
            "Summary"
        );
    }
}
