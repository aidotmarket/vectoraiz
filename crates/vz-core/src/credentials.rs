//! The only credential-store boundary. Never format store errors or secrets.
use crate::cloud_gate::Provider;
use zeroize::Zeroizing;

pub struct Secret(Zeroizing<String>);
impl Secret {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
    pub fn last4(&self) -> String {
        self.0
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredError {
    Missing,
    Denied,
    Unavailable,
}
pub const SERVICE: &str = "market.ai.vectoraiz";

/// Called once by desktop startup, before any credential use. No fallback store.
pub fn init() -> Result<(), CredError> {
    static INIT: std::sync::OnceLock<Result<(), CredError>> = std::sync::OnceLock::new();
    *INIT.get_or_init(|| {
        #[cfg(target_os = "macos")]
        let store = apple_native_keyring_store::keychain::Store::new().map_err(map_error)?;
        #[cfg(windows)]
        let store = windows_native_keyring_store::Store::new().map_err(map_error)?;
        #[cfg(any(target_os = "macos", windows))]
        {
            keyring_core::set_default_store(store);
            Ok(())
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        Err(CredError::Unavailable)
    })
}
fn map_error(error: keyring_core::Error) -> CredError {
    match error {
        keyring_core::Error::NoEntry => CredError::Missing,
        keyring_core::Error::NoStorageAccess(_) => CredError::Denied,
        #[cfg(target_os = "macos")]
        keyring_core::Error::PlatformFailure(error)
            if error
                .downcast_ref::<security_framework::base::Error>()
                .is_some_and(|e| matches!(e.code(), -128 | -25293 | -25308)) =>
        {
            CredError::Denied
        }
        _ => CredError::Unavailable,
    }
}
fn entry(provider: Provider) -> Result<keyring_core::Entry, CredError> {
    #[cfg(windows)]
    {
        // Persist across logons, on this computer only.
        keyring_core::Entry::new_with_modifiers(
            SERVICE,
            provider.id(),
            &std::collections::HashMap::from([("persistence", "Local")]),
        )
        .map_err(map_error)
    }
    #[cfg(not(windows))]
    keyring_core::Entry::new(SERVICE, provider.id()).map_err(map_error)
}
/// Synchronous trait deliberately runs only on the blocking pool. The caller
/// serializes all operations, including Windows, with its cloud_lock.
pub trait Store: Send + Sync {
    fn store(&self, provider: Provider, key: &Secret) -> Result<(), CredError>;
    fn load(&self, provider: Provider) -> Result<Secret, CredError>;
    fn remove(&self, provider: Provider) -> Result<(), CredError>;
}
pub struct NativeStore;
impl Store for NativeStore {
    fn store(&self, provider: Provider, key: &Secret) -> Result<(), CredError> {
        entry(provider)?
            .set_password(key.expose())
            .map_err(map_error)
    }
    fn load(&self, provider: Provider) -> Result<Secret, CredError> {
        entry(provider)?
            .get_password()
            .map(Secret::new)
            .map_err(map_error)
    }
    fn remove(&self, provider: Provider) -> Result<(), CredError> {
        match entry(provider)?.delete_credential().map_err(map_error) {
            Err(CredError::Missing) => Ok(()),
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    #[cfg(target_os = "macos")]
    fn security(args: &[&str]) -> std::process::Output {
        Command::new("security").args(args).output().unwrap()
    }
    #[test]
    fn native_store_os_round_trip_and_process_restart() {
        if let Ok(phase) = std::env::var("VZ_CREDENTIAL_TEST_PHASE") {
            init().unwrap();
            let p = Provider::Anthropic;
            match phase.as_str() {
                "store" => NativeStore
                    .store(p, &Secret::new("sk-test-CI-FAKE-1234".into()))
                    .unwrap(),
                "load" => assert!(NativeStore.load(p).unwrap().last4() == "1234"),
                "remove" => NativeStore.remove(p).unwrap(),
                _ => panic!("unknown phase"),
            }
            return;
        }
        if std::env::var_os("CI").is_none() {
            eprintln!("SKIP OS credential round trip locally: disposable CI user required; no change to the developer's password store");
            return;
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            eprintln!("SKIP OS credential round trip: native desktop platforms only");
            return;
        }
        #[cfg(target_os = "macos")]
        let _temporary_keychain = {
            struct Keychain {
                path: String,
                previous: String,
            }
            impl Drop for Keychain {
                fn drop(&mut self) {
                    let _ = security(&["default-keychain", "-d", "user", "-s", &self.previous]);
                    let _ = security(&["delete-keychain", &self.path]);
                }
            }
            let previous = security(&["default-keychain", "-d", "user"]);
            assert!(previous.status.success());
            let previous = String::from_utf8(previous.stdout)
                .unwrap()
                .trim()
                .trim_matches('"')
                .to_owned();
            let path = std::env::temp_dir()
                .join(format!("vz-ci-{}.keychain-db", uuid::Uuid::new_v4()))
                .to_string_lossy()
                .into_owned();
            let created = security(&["create-keychain", "-p", "vz-disposable-ci-test", &path]);
            if !created.status.success() {
                eprintln!(
                    "SKIP macOS temporary keychain creation: {}",
                    String::from_utf8_lossy(&created.stderr)
                );
                return;
            }
            let guard = Keychain { path, previous };
            let unlocked = security(&[
                "unlock-keychain",
                "-p",
                "vz-disposable-ci-test",
                &guard.path,
            ]);
            if !unlocked.status.success() {
                eprintln!(
                    "SKIP macOS temporary keychain unlock: {}",
                    String::from_utf8_lossy(&unlocked.stderr)
                );
                return;
            }
            assert!(
                security(&["default-keychain", "-d", "user", "-s", &guard.path])
                    .status
                    .success()
            );
            guard
        };
        let child = |phase: &str| {
            let result = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "credentials::tests::native_store_os_round_trip_and_process_restart",
                    "--nocapture",
                ])
                .env("VZ_CREDENTIAL_TEST_PHASE", phase)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "credential child failed (phase {phase}); no secret output retained"
            );
        };
        child("store");
        child("load");
        #[cfg(target_os = "macos")]
        assert!(security(&[
            "find-generic-password",
            "-s",
            SERVICE,
            "-a",
            "anthropic",
            &_temporary_keychain.path
        ])
        .status
        .success());
        #[cfg(windows)]
        {
            let result = Command::new("cmdkey")
                .args(["/list:anthropic.market.ai.vectoraiz"])
                .output()
                .unwrap();
            assert!(result.status.success());
            assert!(
                String::from_utf8_lossy(&result.stdout).contains("anthropic.market.ai.vectoraiz")
            );
        }
        child("remove");
        #[cfg(target_os = "macos")]
        assert!(!security(&[
            "find-generic-password",
            "-s",
            SERVICE,
            "-a",
            "anthropic",
            &_temporary_keychain.path
        ])
        .status
        .success());
        #[cfg(windows)]
        {
            let result = Command::new("cmdkey")
                .args(["/list:anthropic.market.ai.vectoraiz"])
                .output()
                .unwrap();
            assert!(
                !String::from_utf8_lossy(&result.stdout).contains("anthropic.market.ai.vectoraiz")
            );
        }
    }
}
