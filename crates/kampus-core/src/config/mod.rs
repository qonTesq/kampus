//! `school.yaml` and `policy.yaml` (version 1): typed schemas and startup
//! validation. Invalid configuration stops the server before it listens.
//!
//! Secrets are never stored here: fields ending in `_env` name environment
//! variables that must be set at startup.

mod policy;
mod school;
mod validate;

use std::collections::BTreeMap;
use std::path::Path;

pub use policy::{
    AuditPolicy, ClientRegistration, ClientTrust, Effect, PolicyConfig, PolicyLimits,
    RegisteredClient, Rule,
};
pub use school::{
    AuthConfig, AuthMode, BannerConfig, CanvasConfig, DeviceConfig, DeviceSticker, ForwardedHeader,
    GtfsValidity, HoursConfig, LibCalConfig, LibCalLocation, Library, MascotConfig, ParserProfile,
    SandboxConfig, SchoolConfig, SchoolInfo, ServerConfig, ServiceWindow, TransitConfig,
    TransitProvider, TransitRoute, TransitStop, TrustedProxy, UpstreamLimits, parse_range,
};
pub use validate::{ConfigIssue, Env};

use crate::contracts::Operation;

/// Configuration failure with safe, field-level diagnostics.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("cannot parse {path}: {message}")]
    Parse { path: String, message: String },
    #[error("invalid configuration:\n{}", format_issues(.0))]
    Invalid(Vec<ConfigIssue>),
}

fn format_issues(issues: &[ConfigIssue]) -> String {
    issues
        .iter()
        .map(|i| format!("  - {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn read(path: &Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn parse<T: serde::de::DeserializeOwned>(path: &str, text: &str) -> Result<T, ConfigError> {
    serde_saphyr::from_str(text).map_err(|e| ConfigError::Parse {
        path: path.to_owned(),
        message: e.to_string(),
    })
}

impl SchoolConfig {
    pub fn from_yaml(text: &str) -> Result<Self, ConfigError> {
        parse("school.yaml", text)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        parse(&path.display().to_string(), &read(path)?)
    }

    /// Validate structure, URLs, sandbox gates and secret references.
    pub fn validate(&self, env: &Env<'_>) -> Result<(), ConfigError> {
        let issues = school::validate(self, env);
        if issues.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::Invalid(issues))
        }
    }
}

impl PolicyConfig {
    pub fn from_yaml(text: &str) -> Result<Self, ConfigError> {
        parse("policy.yaml", text)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        parse(&path.display().to_string(), &read(path)?)
    }

    /// Validate against the school it belongs to and the registered tools.
    pub fn validate(
        &self,
        school: &SchoolConfig,
        known_tools: &BTreeMap<String, Operation>,
        env: &Env<'_>,
    ) -> Result<(), ConfigError> {
        let issues = policy::validate(self, school, known_tools, env);
        if issues.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::Invalid(issues))
        }
    }
}

/// The operation of each of the seven tools, for tests that validate a
/// policy without registering modules.
#[cfg(test)]
pub(crate) fn tool_operation(tool: &str) -> Operation {
    match tool {
        "confirm_booking" => Operation::Confirm,
        "book_study_room" => Operation::Propose,
        _ => Operation::Read,
    }
}

#[cfg(test)]
mod tests;
