use super::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};
#[derive(Default)]
struct Fake {
    data: RefCell<HashMap<String, Vec<u8>>>,
    reads: Cell<usize>,
    legacy_reads: Cell<usize>,
    writes: Cell<usize>,
    denied: Cell<bool>,
    fail_write: Cell<bool>,
}
impl Backend for Fake {
    fn load(&self, instance: &str) -> anyhow::Result<Option<Vec<u8>>> {
        self.reads.set(self.reads.get() + 1);
        anyhow::ensure!(!self.denied.get(), "Canceled");
        Ok(self.data.borrow().get(instance).cloned())
    }
    fn legacy(&self, _: &str, name: &str) -> anyhow::Result<String> {
        self.legacy_reads.set(self.legacy_reads.get() + 1);
        Ok(format!("old-{name}"))
    }
    fn save(&self, instance: &str, data: &[u8]) -> anyhow::Result<()> {
        anyhow::ensure!(!self.fail_write.get(), "Write denied");
        self.writes.set(self.writes.get() + 1);
        self.data.borrow_mut().insert(instance.into(), data.into());
        Ok(())
    }
}
#[test]
fn one_unlock_reads_all_credentials_and_migration_only_happens_on_save() {
    let backend = Fake::default();
    let mut vault = Vault::default();
    for name in NAMES {
        assert_eq!(
            vault.read(&backend, "instance", name).unwrap(),
            format!("old-{name}")
        );
    }
    assert_eq!(backend.reads.get(), 1);
    assert_eq!(backend.legacy_reads.get(), 3);
    assert_eq!(backend.writes.get(), 0);
    vault
        .write(&backend, "instance", "gitlab-token", "new-token")
        .unwrap();
    let mut restarted = Vault::default();
    assert_eq!(
        restarted
            .read(&backend, "instance", "gitlab-token")
            .unwrap(),
        "new-token"
    );
    assert_eq!(
        restarted.read(&backend, "instance", "cf-secret").unwrap(),
        "old-cf-secret"
    );
    assert_eq!(backend.reads.get(), 2);
    assert_eq!(backend.legacy_reads.get(), 3);
}
#[test]
fn canceled_authentication_never_falls_back_to_legacy_or_caches_failure() {
    let backend = Fake::default();
    backend.denied.set(true);
    let mut vault = Vault::default();
    assert!(vault.read(&backend, "instance", "gitlab-token").is_err());
    assert_eq!(backend.legacy_reads.get(), 0);
    assert!(vault.unlocked.is_empty());
    backend.denied.set(false);
    assert!(vault.read(&backend, "instance", "gitlab-token").is_ok());
}
#[test]
fn failed_save_keeps_previous_credentials_and_empty_values_do_not_resurrect_legacy() {
    let backend = Fake::default();
    let mut vault = Vault::default();
    vault
        .write(&backend, "instance", "gitlab-token", "new-token")
        .unwrap();
    backend.fail_write.set(true);
    assert!(
        vault
            .write(&backend, "instance", "gitlab-token", "rejected-token")
            .is_err()
    );
    assert_eq!(
        vault.read(&backend, "instance", "gitlab-token").unwrap(),
        "new-token"
    );
    backend.fail_write.set(false);
    for name in NAMES {
        vault.write(&backend, "instance", name, "").unwrap();
    }
    let mut restarted = Vault::default();
    for name in NAMES {
        assert_eq!(restarted.read(&backend, "instance", name).unwrap(), "");
    }
    assert_eq!(backend.legacy_reads.get(), 3);
}
#[test]
fn instances_are_isolated_and_corrupted_entries_fail_closed() {
    let backend = Fake::default();
    let mut vault = Vault::default();
    vault
        .write(&backend, "a", "gitlab-token", "token-a")
        .unwrap();
    assert_eq!(
        vault.read(&backend, "b", "gitlab-token").unwrap(),
        "old-gitlab-token"
    );
    backend
        .data
        .borrow_mut()
        .insert("c".into(), br#"{"version":2,"values":{}}"#.to_vec());
    let count = backend.legacy_reads.get();
    assert!(vault.read(&backend, "c", "gitlab-token").is_err());
    assert_eq!(backend.legacy_reads.get(), count);
    assert!(vault.read(&backend, "a", "unknown").is_err());
}

#[test]
fn all_credentials_are_saved_in_one_atomic_keychain_update() {
    let backend = Fake::default();
    let mut vault = Vault::default();
    vault
        .write_many(
            &backend,
            "instance",
            &[
                ("gitlab-token", "new-token"),
                ("cf-id", "new-id"),
                ("cf-secret", "new-secret"),
            ],
        )
        .unwrap();
    assert_eq!(backend.writes.get(), 1);
    backend.fail_write.set(true);
    assert!(
        vault
            .write_many(
                &backend,
                "instance",
                &[("gitlab-token", "bad"), ("cf-secret", "bad")]
            )
            .is_err()
    );
    assert_eq!(
        vault.read(&backend, "instance", "gitlab-token").unwrap(),
        "new-token"
    );
    assert_eq!(
        vault.read(&backend, "instance", "cf-secret").unwrap(),
        "new-secret"
    );
}
