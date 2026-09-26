use std::collections::BTreeSet;
use std::fmt;

use url::Url;

/// One safe, field-level validation problem. Never contains secret values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIssue {
    pub field: String,
    pub message: String,
}

impl fmt::Display for ConfigIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

/// Startup environment used by validation (injected so tests need no
/// process-wide environment changes).
pub struct Env<'a> {
    /// Returns whether a non-empty environment variable is set.
    pub has_var: &'a dyn Fn(&str) -> bool,
    /// Whether the binary was compiled with the `sandbox` feature.
    pub sandbox_compiled: bool,
}

impl fmt::Debug for Env<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Env")
            .field("sandbox_compiled", &self.sandbox_compiled)
            .finish_non_exhaustive()
    }
}

impl Env<'static> {
    /// The real process environment and build.
    pub fn process() -> Self {
        Env {
            has_var: &|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()),
            sandbox_compiled: crate::SANDBOX_COMPILED,
        }
    }
}

/// Collects issues while walking a config tree.
#[derive(Debug, Default)]
pub(super) struct Issues(pub(super) Vec<ConfigIssue>);

impl Issues {
    pub(super) fn push(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.0.push(ConfigIssue {
            field: field.into(),
            message: message.into(),
        });
    }

    /// `https`, no credentials, no placeholder host; `http` only on loopback.
    pub(super) fn url(&mut self, field: &str, url: &Url) {
        if !url.username().is_empty() || url.password().is_some() {
            self.push(field, "must not embed credentials");
        }
        let Some(host) = url.host_str() else {
            self.push(field, "must have a host");
            return;
        };
        if host.ends_with(".invalid") || host.contains("change-me") {
            self.push(
                field,
                "placeholder host; set the real value before starting",
            );
        }
        match url.scheme() {
            "https" => {}
            "http" if is_loopback_host(host) => {}
            _ => self.push(field, "must use https (http only for loopback)"),
        }
    }

    /// Loopback-only URL (sandbox fakes).
    pub(super) fn loopback_url(&mut self, field: &str, url: &Url) {
        if !url.host_str().is_some_and(is_loopback_host) {
            self.push(field, "must point at a loopback host");
        }
        self.url(field, url);
    }

    /// An environment-variable secret reference that must be set.
    pub(super) fn secret_env(&mut self, field: &str, name: &str, env: &Env<'_>) {
        let well_formed = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            && !name.as_bytes()[0].is_ascii_digit();
        if !well_formed {
            self.push(
                field,
                "must name an environment variable ([A-Z_][A-Z0-9_]*)",
            );
        } else if !(env.has_var)(name) {
            self.push(field, format!("environment variable {name} is not set"));
        }
    }

    pub(super) fn id(&mut self, field: &str, id: &str) {
        let ok = (1..=64).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'));
        if !ok {
            self.push(field, "must be 1-64 chars of [A-Za-z0-9_.:-]");
        }
    }

    pub(super) fn unique<'a>(&mut self, field: &str, values: impl IntoIterator<Item = &'a str>) {
        let mut seen = BTreeSet::new();
        for value in values {
            if !seen.insert(value) {
                self.push(field, format!("duplicate id {value}"));
            }
        }
    }
}

pub(super) fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
}
