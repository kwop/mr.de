//! Safe Security.framework APIs; no subprocess receives credentials.
use anyhow::Context;
use core_foundation::data::CFData;
use security_framework::{
    access_control::{ProtectionMode, SecAccessControl},
    item::{ItemClass, ItemSearchOptions, ItemUpdateOptions, ItemUpdateValue, update_item},
    os::macos::code_signing::{Flags, SecCode, SecRequirement},
    passwords::{
        AccessControlOptions, PasswordOptions, generic_password, set_generic_password_options,
    },
};

const SERVICE: &str = "io.github.kwop.mrde.credentials.v1";
const NOT_FOUND: i32 = -25300;

pub(super) fn is_provisioned_app() -> bool {
    // The packaging script provides a profile authorizing these entitlements. Plain
    // `cargo run` remains compatible with existing keyring entries and ad-hoc signing.
    let requirement: Result<SecRequirement, _> = r#"anchor apple generic and identifier "io.github.kwop.mrde" and entitlement["com.apple.application-identifier"] exists and entitlement["keychain-access-groups"] exists"#.parse();
    match (SecCode::for_self(Flags::NONE), requirement) {
        (Ok(code), Ok(requirement)) => code.check_validity(Flags::NONE, &requirement).is_ok(),
        _ => false,
    }
}
fn query(instance: &str) -> PasswordOptions {
    let mut options = PasswordOptions::new_generic_password(SERVICE, instance);
    options.use_protected_keychain();
    options
}
fn failure(error: security_framework::base::Error) -> anyhow::Error {
    match error.code() {
        -128 => anyhow::anyhow!("Keychain unlock was canceled"),
        -34018 => anyhow::anyhow!(
            "This build is missing the macOS Keychain entitlement. Launch the provisioned mr.de.app."
        ),
        -25308 => anyhow::anyhow!(
            "Keychain authentication is unavailable. Unlock your Mac and try again."
        ),
        code => anyhow::anyhow!("Protected Keychain operation failed (macOS error {code})"),
    }
}
pub(super) struct Native;
impl super::Backend for Native {
    fn load(&self, instance: &str) -> anyhow::Result<Option<Vec<u8>>> {
        match generic_password(query(instance)) {
            Ok(data) => Ok(Some(data)),
            Err(error) if error.code() == NOT_FOUND => Ok(None),
            Err(error) => Err(failure(error)),
        }
    }
    fn save(&self, instance: &str, data: &[u8]) -> anyhow::Result<()> {
        // Update only the bytes. Preserve the existing ACL and never delete/recreate an
        // item on failure. In particular, never use an unprotected upsert after a miss.
        let mut search = ItemSearchOptions::new();
        search
            .class(ItemClass::generic_password())
            .service(SERVICE)
            .account(instance)
            .ignore_legacy_keychains();
        let mut update = ItemUpdateOptions::new();
        update.set_value(ItemUpdateValue::Data(CFData::from_buffer(data)));
        match update_item(&search, &update) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == NOT_FOUND => {
                let access = SecAccessControl::create_with_protection(
                    Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
                    AccessControlOptions::USER_PRESENCE.bits(),
                )
                .context("Cannot create the Touch ID access policy")?;
                let mut options = query(instance);
                options.set_access_control(access);
                set_generic_password_options(data, options).map_err(failure)
            }
            Err(error) => Err(failure(error)),
        }
    }
    fn legacy(&self, instance: &str, name: &str) -> anyhow::Result<String> {
        super::legacy_read(instance, name)
    }
}
