//! Credential routing: ordinary Cargo binaries keep the legacy keyring backend;
//! the provisioned app uses one user-presence-protected item per GitLab instance.
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Mutex, OnceLock},
};

mod native;
const NAMES: [&str; 3] = ["gitlab-token", "cf-id", "cf-secret"];

// Intentionally no Debug: this type contains credentials.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Bundle {
    version: u8,
    values: BTreeMap<String, String>,
}
impl Bundle {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let value: Self =
            serde_json::from_slice(bytes).context("Invalid protected credential format")?;
        anyhow::ensure!(
            value.version == 1,
            "Unsupported protected credential version"
        );
        Ok(value)
    }
}
trait Backend {
    fn load(&self, instance: &str) -> anyhow::Result<Option<Vec<u8>>>;
    fn save(&self, instance: &str, data: &[u8]) -> anyhow::Result<()>;
    fn legacy(&self, instance: &str, name: &str) -> anyhow::Result<String>;
}
#[derive(Default)]
struct Vault {
    unlocked: HashMap<String, Bundle>,
}
impl Vault {
    fn unlock(&mut self, backend: &impl Backend, instance: &str) -> anyhow::Result<&Bundle> {
        if !self.unlocked.contains_key(instance) {
            // Only a genuinely absent item permits legacy lookup. Cancellation, missing
            // entitlements, locked Keychain, and authentication failures never fall back.
            let bundle = match backend.load(instance)? {
                Some(data) => Bundle::decode(&data)?,
                None => {
                    let mut values = BTreeMap::new();
                    for name in NAMES {
                        values.insert(name.into(), backend.legacy(instance, name)?);
                    }
                    Bundle { version: 1, values }
                }
            };
            self.unlocked.insert(instance.into(), bundle);
        }
        self.unlocked
            .get(instance)
            .context("Credential cache unavailable")
    }
    fn read(
        &mut self,
        backend: &impl Backend,
        instance: &str,
        name: &str,
    ) -> anyhow::Result<String> {
        anyhow::ensure!(NAMES.contains(&name), "Unknown credential name");
        Ok(self
            .unlock(backend, instance)?
            .values
            .get(name)
            .cloned()
            .unwrap_or_default())
    }
    fn write_many(
        &mut self,
        backend: &impl Backend,
        instance: &str,
        updates: &[(&str, &str)],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            updates.iter().all(|(name, _)| NAMES.contains(name)),
            "Unknown credential name"
        );
        let mut bundle = self.unlock(backend, instance)?.clone();
        for (name, value) in updates {
            bundle.values.insert((*name).into(), (*value).into());
        }
        // An empty bundle is a tombstone: old legacy secrets must never reappear.
        // Persist all changes atomically before replacing the unlocked cache.
        backend.save(instance, &serde_json::to_vec(&bundle)?)?;
        self.unlocked.insert(instance.into(), bundle);
        Ok(())
    }
    #[cfg(test)]
    fn write(
        &mut self,
        backend: &impl Backend,
        instance: &str,
        name: &str,
        value: &str,
    ) -> anyhow::Result<()> {
        self.write_many(backend, instance, &[(name, value)])
    }
}
fn protected() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(native::is_provisioned_app)
}
fn vault() -> &'static Mutex<Vault> {
    static VAULT: OnceLock<Mutex<Vault>> = OnceLock::new();
    VAULT.get_or_init(Mutex::default)
}
/// Called only from background credential tasks; values must never be logged.
pub fn read_secret(instance: &str, name: &str) -> anyhow::Result<String> {
    if protected() {
        vault()
            .lock()
            .map_err(|_| anyhow::anyhow!("Credential cache unavailable"))?
            .read(&native::Native, instance, name)
    } else {
        legacy_read(instance, name)
    }
}
pub fn write_secret(instance: &str, name: &str, value: &str) -> anyhow::Result<()> {
    write_secrets(instance, &[(name, value)])
}
/// Save the GitLab and Cloudflare values together, with one protected-item update.
pub fn write_secrets(instance: &str, values: &[(&str, &str)]) -> anyhow::Result<()> {
    if protected() {
        vault()
            .lock()
            .map_err(|_| anyhow::anyhow!("Credential cache unavailable"))?
            .write_many(&native::Native, instance, values)
    } else {
        let old = values
            .iter()
            .map(|(name, _)| legacy_read(instance, name))
            .collect::<anyhow::Result<Vec<_>>>()?;
        for (index, (name, value)) in values.iter().enumerate() {
            if let Err(error) = legacy_write(instance, name, value) {
                for ((name, _), value) in values[..index].iter().zip(&old) {
                    let _ = legacy_write(instance, name, value);
                }
                return Err(error);
            }
        }
        Ok(())
    }
}

/// Values must never be logged or serialized into application configuration.
fn legacy_read(instance: &str, name: &str) -> anyhow::Result<String> {
    let entry = keyring::Entry::new("mrde", &format!("{instance}:{name}"))?;
    match entry.get_password() {
        Ok(value) => Ok(value),
        Err(keyring::Error::NoEntry) => Ok(String::new()),
        Err(error) => Err(error).context("Cannot read macOS Keychain"),
    }
}
fn legacy_write(instance: &str, name: &str, value: &str) -> anyhow::Result<()> {
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
mod tests;
