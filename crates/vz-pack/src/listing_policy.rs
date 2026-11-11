//! Machine policy shared by every native listing entry point.
//!
//! This first C1 slice deliberately grants no packaging capability: the staged
//! review/sanitizer is not complete. Policy permission alone is never a review.
use serde::Serialize;
use std::sync::{Arc, Mutex, OnceLock};
use tokio_util::sync::CancellationToken;

/// Exposure flag. No environment variable or user preference can enable this
/// incomplete journey. Search, index and answers do not consult it.
pub const LISTING_AUTHORITY_PREVIEW_V1: bool = false;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedPolicy {
    Unmanaged,
    Allowed,
    Denied,
    Invalid,
}

/// Injectable machine-policy source. Errors must map to Invalid, not Unmanaged.
pub trait PolicySource: Send + Sync {
    fn read(&self) -> ManagedPolicy;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ListingError {
    PolicyDenied,
    PolicyUnreadable,
    PreviewDisabled,
    ReviewRequired,
    StaleGeneration,
}
impl std::fmt::Display for ListingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::PolicyDenied => "Listing is disabled by your organisation. Contact IT.",
            Self::PolicyUnreadable => "The managed listing policy cannot be read or is invalid. Listing is disabled. Contact IT.",
            Self::PreviewDisabled => "Listing preview is not available in this build.",
            Self::ReviewRequired => "A current purchaser review is required. Listing is unavailable until review support is complete.",
            Self::StaleGeneration => "Listing policy, source or reviewed generation changed. Start a new purchaser review.",
        })
    }
}
impl std::error::Error for ListingError {}

#[derive(Clone, Debug, Serialize)]
pub struct ListingStatus {
    pub listing_authority_preview_v1: bool,
    pub managed_policy: ManagedPolicy,
    pub policy_generation: u64,
    pub available: bool,
    pub code: ListingError,
    pub reason: String,
}

struct PolicyState {
    value: ManagedPolicy,
    generation: u64,
    cancel: CancellationToken,
}

pub struct ListingPolicy {
    source: Arc<dyn PolicySource>,
    state: Mutex<PolicyState>,
}

/// A policy lease, not permission to deliver bytes. All jobs in a policy
/// generation share one cancellation token. Re-enabling never revives a lease.
pub struct PolicyLease {
    generation: u64,
    cancel: CancellationToken,
}
impl PolicyLease {
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
    pub fn check(&self, policy: &ListingPolicy) -> Result<(), ListingError> {
        let mut state = policy.state.lock().unwrap();
        policy.refresh_locked(&mut state);
        if self.cancel.is_cancelled() || state.generation != self.generation {
            return Err(ListingError::StaleGeneration);
        }
        policy_allowed(state.value)
    }
}

fn policy_allowed(value: ManagedPolicy) -> Result<(), ListingError> {
    match value {
        ManagedPolicy::Unmanaged | ManagedPolicy::Allowed => Ok(()),
        ManagedPolicy::Denied => Err(ListingError::PolicyDenied),
        ManagedPolicy::Invalid => Err(ListingError::PolicyUnreadable),
    }
}

impl ListingPolicy {
    pub fn new(source: Arc<dyn PolicySource>) -> Self {
        let value = source.read();
        Self {
            source,
            state: Mutex::new(PolicyState {
                value,
                generation: 0,
                cancel: CancellationToken::new(),
            }),
        }
    }

    fn refresh_locked(&self, state: &mut PolicyState) {
        // Serialize source reads as well as snapshots: a delayed old read must
        // not overwrite a newer deny. Never acquire a pack mutex from here.
        let next = self.source.read();
        if next != state.value {
            state.cancel.cancel();
            state.cancel = CancellationToken::new();
            state.value = next;
            state.generation = state
                .generation
                .checked_add(1)
                .expect("policy generation overflow");
        }
    }

    pub fn refresh(&self) {
        self.refresh_locked(&mut self.state.lock().unwrap());
    }

    /// Take this snapshot before a future per-pack mutex, then recheck the lease
    /// before each write/promotion. It cannot bypass the review entry gate.
    pub fn policy_lease(&self) -> Result<PolicyLease, ListingError> {
        let mut state = self.state.lock().unwrap();
        self.refresh_locked(&mut state);
        policy_allowed(state.value)?;
        Ok(PolicyLease {
            generation: state.generation,
            cancel: state.cancel.clone(),
        })
    }

    pub fn status(&self) -> ListingStatus {
        self.status_for_flag(LISTING_AUTHORITY_PREVIEW_V1)
    }

    fn status_for_flag(&self, enabled: bool) -> ListingStatus {
        let mut state = self.state.lock().unwrap();
        self.refresh_locked(&mut state);
        let code = match policy_allowed(state.value) {
            Err(error) => error,
            Ok(()) if !enabled => ListingError::PreviewDisabled,
            // Enabling an exposure flag must never reopen the old byte copier.
            Ok(()) => ListingError::ReviewRequired,
        };
        ListingStatus {
            listing_authority_preview_v1: enabled,
            managed_policy: state.value,
            policy_generation: state.generation,
            available: false,
            code,
            reason: code.to_string(),
        }
    }

    pub fn require_listing(&self) -> Result<(), ListingError> {
        Err(self.status().code)
    }

    /// Availability check for the new review commands. Managed denial always
    /// precedes feature availability. This grants no transfer capability.
    pub fn require_preview(&self) -> Result<(), ListingError> {
        let lease = self.policy_lease()?;
        if !LISTING_AUTHORITY_PREVIEW_V1 {
            return Err(ListingError::PreviewDisabled);
        }
        lease.check(self)
    }
}

struct OsPolicy;
impl PolicySource for OsPolicy {
    fn read(&self) -> ManagedPolicy {
        read_os_policy()
    }
}

/// One process-wide controller for desktop, CLI and tools. The background
/// fallback refresh catches idle policy changes; entries always refresh too.
pub fn system() -> &'static ListingPolicy {
    static POLICY: OnceLock<ListingPolicy> = OnceLock::new();
    static MONITOR: OnceLock<()> = OnceLock::new();
    let policy = POLICY.get_or_init(|| ListingPolicy::new(Arc::new(OsPolicy)));
    MONITOR.get_or_init(|| {
        #[cfg(windows)]
        std::thread::spawn(move || watch_registry(policy));
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            policy.refresh();
        });
    });
    policy
}

/// Windows exposes registry notifications. Watch the parent subtree so creating
/// or deleting the policy key is observed too. Polling remains active if opening
/// the notification handle or waiting fails. macOS CFPreferences provides no
/// corresponding public per-key notification; it uses the refresh fallback.
#[cfg(windows)]
fn watch_registry(policy: &'static ListingPolicy) {
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{
            RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY_LOCAL_MACHINE, KEY_NOTIFY,
            KEY_WOW64_64KEY, REG_NOTIFY_CHANGE_LAST_SET, REG_NOTIFY_CHANGE_NAME,
        },
    };
    let path = "SOFTWARE\\Policies"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: a query/notification-only HKLM handle, owned by this thread and
    // closed on wait failure. Synchronous notification uses no event handle.
    unsafe {
        let mut key = std::ptr::null_mut();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            0,
            KEY_NOTIFY | KEY_WOW64_64KEY,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return;
        }
        loop {
            if RegNotifyChangeKeyValue(
                key,
                1,
                REG_NOTIFY_CHANGE_LAST_SET | REG_NOTIFY_CHANGE_NAME,
                std::ptr::null_mut(),
                0,
            ) != ERROR_SUCCESS
            {
                break;
            }
            policy.refresh();
        }
        RegCloseKey(key);
    }
}

#[cfg(target_os = "macos")]
fn read_os_policy() -> ManagedPolicy {
    use core_foundation_sys::{
        base::{CFGetTypeID, CFRelease},
        number::{CFBooleanGetTypeID, CFBooleanGetValue},
        preferences::{
            CFPreferencesAppSynchronize, CFPreferencesAppValueIsForced, CFPreferencesCopyAppValue,
        },
        string::{kCFStringEncodingUTF8, CFStringCreateWithCString},
    };
    // SAFETY: literal C strings; every owned CF object is released exactly once.
    // Only a forced Boolean is a policy; ordinary user defaults cannot allow it.
    unsafe {
        let app = CFStringCreateWithCString(
            std::ptr::null(),
            c"com.vectoraiz.native".as_ptr(),
            kCFStringEncodingUTF8,
        );
        let key = CFStringCreateWithCString(
            std::ptr::null(),
            c"listing_allowed".as_ptr(),
            kCFStringEncodingUTF8,
        );
        if app.is_null() || key.is_null() {
            if !app.is_null() {
                CFRelease(app.cast());
            }
            if !key.is_null() {
                CFRelease(key.cast());
            }
            return ManagedPolicy::Invalid;
        }
        let synced = CFPreferencesAppSynchronize(app) != 0;
        let forced = CFPreferencesAppValueIsForced(key, app) != 0;
        let value = CFPreferencesCopyAppValue(key, app);
        let result = if !synced {
            ManagedPolicy::Invalid
        } else if !forced {
            ManagedPolicy::Unmanaged
        } else if value.is_null() || CFGetTypeID(value) != CFBooleanGetTypeID() {
            ManagedPolicy::Invalid
        } else if CFBooleanGetValue(value.cast()) {
            ManagedPolicy::Allowed
        } else {
            ManagedPolicy::Denied
        };
        if !value.is_null() {
            CFRelease(value);
        }
        CFRelease(key.cast());
        CFRelease(app.cast());
        result
    }
}

#[cfg(windows)]
fn read_os_policy() -> ManagedPolicy {
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE,
            KEY_WOW64_64KEY, REG_DWORD,
        },
    };
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let path = wide(r"SOFTWARE\Policies\vectorAIz");
    let name = wide("listing_allowed");
    // SAFETY: buffers and output handles live for each Win32 call. Query only,
    // explicit machine/64-bit view; no HKCU fallback and no ACL mutation.
    unsafe {
        let mut key = std::ptr::null_mut();
        let opened = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            0,
            KEY_QUERY_VALUE | KEY_WOW64_64KEY,
            &mut key,
        );
        if opened == ERROR_FILE_NOT_FOUND || opened == ERROR_PATH_NOT_FOUND {
            return ManagedPolicy::Unmanaged;
        }
        if opened != ERROR_SUCCESS {
            return ManagedPolicy::Invalid;
        }
        let mut kind = 0;
        let mut value = 0_u32;
        let mut size = 4;
        let read = RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null(),
            &mut kind,
            (&mut value as *mut u32).cast(),
            &mut size,
        );
        RegCloseKey(key);
        if read == ERROR_FILE_NOT_FOUND {
            ManagedPolicy::Unmanaged
        } else if read != ERROR_SUCCESS || kind != REG_DWORD || size != 4 {
            ManagedPolicy::Invalid
        } else {
            match value {
                0 => ManagedPolicy::Denied,
                1 => ManagedPolicy::Allowed,
                _ => ManagedPolicy::Invalid,
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn read_os_policy() -> ManagedPolicy {
    // Unsupported platforms cannot claim managed policy enforcement.
    ManagedPolicy::Invalid
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exposure_flag_never_overrides_policy_or_missing_review() {
        struct Fixed(ManagedPolicy);
        impl PolicySource for Fixed {
            fn read(&self) -> ManagedPolicy {
                self.0
            }
        }
        for (value, error) in [
            (ManagedPolicy::Denied, ListingError::PolicyDenied),
            (ManagedPolicy::Invalid, ListingError::PolicyUnreadable),
        ] {
            let policy = ListingPolicy::new(Arc::new(Fixed(value)));
            for enabled in [false, true] {
                assert_eq!(policy.status_for_flag(enabled).code, error);
            }
        }
        let policy = ListingPolicy::new(Arc::new(Fixed(ManagedPolicy::Unmanaged)));
        assert_eq!(
            policy.status_for_flag(false).code,
            ListingError::PreviewDisabled
        );
        assert_eq!(
            policy.status_for_flag(true).code,
            ListingError::ReviewRequired
        );
        assert!(!policy.status_for_flag(true).available);
    }
}
