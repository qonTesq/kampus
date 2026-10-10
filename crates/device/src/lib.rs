//! Ephemeral, device-scoped synthetic identity. No credentials leave this crate.
mod endpoint;
mod limits;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aws_lc_rs::{constant_time, digest};
use chrono::{DateTime, Utc};
use kampus_core::audit::SubjectHasher;
use kampus_core::config::{ClientRegistration, SchoolConfig};
use kampus_core::contracts::{AuthKind, IdentityResolver, Principal, ResolvedIdentity, Role};
use kampus_core::dispatch::Security;
use kampus_core::policy::PolicyEngine;

pub use endpoint::routes;

pub const BINDING_TTL: Duration = Duration::from_mins(2);

/// Injectable monotonic and wall clocks. Only monotonic time grants authority.
pub trait Clock: Send + Sync {
    fn elapsed(&self) -> Duration;
    fn wall(&self) -> DateTime<Utc>;
}

#[derive(Debug)]
pub struct SystemClock(Instant);
impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}
impl Clock for SystemClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
    fn wall(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid device configuration or missing distinct Pi secret")]
pub struct DeviceError;

struct Device {
    id: String,
    client_id: String,
    secret: digest::Digest,
    stickers: BTreeMap<String, String>,
}
struct Binding {
    subject: String,
    deadline: Duration,
    generation: u64,
}
#[derive(Default)]
struct State {
    bindings: BTreeMap<String, Binding>,
    generation: u64,
    limits: limits::Limits,
}

/// One instance per school/process. Construct a new instance to clear bindings.
pub struct DeviceBindings {
    school_id: String,
    devices: Vec<Device>,
    state: Mutex<State>,
    bind_lock: tokio::sync::Mutex<()>,
    clock: Arc<dyn Clock>,
    audit: Arc<dyn kampus_core::contracts::AuditSink>,
    hasher: SubjectHasher,
    policy: Arc<PolicyEngine>,
}
impl fmt::Debug for DeviceBindings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceBindings").finish_non_exhaustive()
    }
}

impl DeviceBindings {
    /// Secrets are read once; restart after rotation. Values are never debugged.
    pub fn new(
        school: &SchoolConfig,
        security: &Security,
        secret: &dyn Fn(&str) -> Option<String>,
        clock: Arc<dyn Clock>,
    ) -> Result<Arc<Self>, DeviceError> {
        let mut devices = Vec::<Device>::new();
        let policy = security.policy.current();
        for config in &school.devices {
            let registered = policy
                .config
                .client_registry
                .iter()
                .find(|c| c.client_id == config.client_id);
            if !registered
                .is_some_and(|c| c.registration == ClientRegistration::Device && !c.approved)
                || devices
                    .iter()
                    .any(|d| d.id == config.device_id || d.client_id == config.client_id)
            {
                return Err(DeviceError);
            }
            let value = secret(&config.bind_secret_env).ok_or(DeviceError)?;
            // A Pi secret is an opaque value, never a JWT (three dotted segments).
            if value.len() < 32
                || value.len() > 512
                || value.contains('.')
                || value.bytes().any(|b| b.is_ascii_whitespace())
            {
                return Err(DeviceError);
            }
            let secret = digest::digest(&digest::SHA256, value.as_bytes());
            if devices.iter().any(|d| {
                constant_time::verify_slices_are_equal(d.secret.as_ref(), secret.as_ref()).is_ok()
            }) {
                return Err(DeviceError);
            }
            let mut stickers = BTreeMap::new();
            for sticker in &config.stickers {
                if !synthetic(&sticker.uid)
                    || !synthetic(&sticker.subject)
                    || sticker.role != Role::Student
                    || stickers
                        .insert(sticker.uid.clone(), sticker.subject.clone())
                        .is_some()
                {
                    return Err(DeviceError);
                }
            }
            devices.push(Device {
                id: config.device_id.clone(),
                client_id: config.client_id.clone(),
                secret,
                stickers,
            });
        }
        Ok(Arc::new(Self {
            school_id: school.school.id.clone(),
            devices,
            state: Mutex::new(State::default()),
            bind_lock: tokio::sync::Mutex::new(()),
            clock,
            audit: Arc::clone(&security.audit),
            hasher: SubjectHasher::ephemeral(&school.school.id).map_err(|_| DeviceError)?,
            policy: Arc::clone(&security.policy),
        }))
    }

    fn authenticate(&self, value: &str) -> Option<&Device> {
        let candidate = digest::digest(&digest::SHA256, value.as_bytes());
        let mut found = None;
        for device in &self.devices {
            if constant_time::verify_slices_are_equal(device.secret.as_ref(), candidate.as_ref())
                .is_ok()
            {
                found = Some(device);
            }
        }
        found
    }
}

fn synthetic(value: &str) -> bool {
    value.starts_with("synthetic:")
        && value.len() > 10
        && value.len() <= 128
        && !value.chars().any(char::is_control)
}

impl IdentityResolver for DeviceBindings {
    fn resolve(&self, principal: &Principal, _now: DateTime<Utc>) -> ResolvedIdentity {
        let mut result = ResolvedIdentity {
            principal: principal.clone(),
            binding_generation: None,
        };
        if principal.auth_kind != AuthKind::Device {
            return result;
        }
        Principal::GUEST_SUBJECT.clone_into(&mut result.principal.subject);
        result.principal.roles = BTreeSet::from([Role::Guest]);
        let Some(device) = self.devices.iter().find(|d| {
            Some(d.id.as_str()) == principal.device_id.as_deref()
                && Some(d.client_id.as_str()) == principal.client_id.as_deref()
        }) else {
            return result;
        };
        let Ok(mut state) = self.state.lock() else {
            return result;
        };
        let now = self.clock.elapsed();
        state.bindings.retain(|_, b| now < b.deadline);
        if let Some(binding) = state.bindings.get(&device.id) {
            result.principal.subject.clone_from(&binding.subject);
            result.principal.roles = BTreeSet::from([Role::Student]);
            result.binding_generation = Some(binding.generation);
        }
        result
    }
}
