use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};
use url::Url;

use super::validate::{Env, Issues};
use crate::contracts::Role;

/// `school.yaml` version 1. Everything school-specific lives here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchoolConfig {
    pub version: u32,
    pub school: SchoolInfo,
    pub server: ServerConfig,
    pub auth: AuthConfig,
    /// Enabled module IDs. Disabled modules keep their schemas and answer
    /// `unsupported_source` after policy checks.
    pub modules: Vec<String>,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub mascot: Option<MascotConfig>,
    #[serde(default)]
    pub libcal: Option<LibCalConfig>,
    #[serde(default)]
    pub banner: Option<BannerConfig>,
    #[serde(default)]
    pub hours: Option<HoursConfig>,
    #[serde(default)]
    pub transit: Option<TransitConfig>,
    #[serde(default)]
    pub canvas: Option<CanvasConfig>,
    #[serde(default)]
    pub devices: Vec<DeviceConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchoolInfo {
    pub id: String,
    pub display_name: String,
    /// IANA timezone, e.g. `America/New_York`.
    pub timezone: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Canonical MCP resource URL, e.g. `https://host/mcp`. Access tokens must
    /// name exactly this URL in `aud` (RFC 8707/9728); there is no alias.
    pub resource_url: Url,
    /// Public MCP listener (behind Caddy).
    pub listen: SocketAddr,
    /// Local admin listener; must be loopback.
    pub admin_listen: SocketAddr,
    /// Reverse proxies whose forwarded client address is believed. Off
    /// unless configured: forwarded headers are ignored by default.
    #[serde(default)]
    pub trusted_proxy: Option<TrustedProxy>,
}

/// Which forwarded header carries the client address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardedHeader {
    /// `CF-Connecting-IP` (Cloudflare Tunnel): one address.
    CfConnectingIp,
    /// `X-Forwarded-For`: the rightmost address that is not a trusted peer.
    XForwardedFor,
}

impl ForwardedHeader {
    /// The lowercase HTTP header name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::CfConnectingIp => "cf-connecting-ip",
            Self::XForwardedFor => "x-forwarded-for",
        }
    }
}

/// `server.trusted_proxy`: used only to key guest rate limits (in memory).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedProxy {
    pub header: ForwardedHeader,
    /// Proxy peer addresses or CIDR ranges (`127.0.0.1`, `10.0.0.0/8`, `::1`).
    pub peers: Vec<String>,
}

impl TrustedProxy {
    /// Whether `ip` is one of the configured proxy peers.
    pub fn trusts(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        self.peers
            .iter()
            .filter_map(|p| parse_range(p))
            .any(|(net, len)| in_range(ip, net, len))
    }

    /// The client address of a connection from `peer`, given every value of
    /// the configured header. Headers from untrusted peers, and values that
    /// do not parse, are ignored in favour of `peer`.
    pub fn client_ip<'a>(&self, peer: IpAddr, values: impl IntoIterator<Item = &'a str>) -> IpAddr {
        if !self.trusts(peer) {
            return peer;
        }
        let hops: Option<Vec<IpAddr>> = values
            .into_iter()
            .flat_map(|v| v.split(','))
            .map(|hop| hop.trim().parse::<IpAddr>().ok())
            .collect();
        let Some(hops) = hops.filter(|h| !h.is_empty()) else {
            return peer;
        };
        match self.header {
            ForwardedHeader::CfConnectingIp => match hops.as_slice() {
                [only] => *only,
                _ => peer,
            },
            ForwardedHeader::XForwardedFor => hops
                .iter()
                .rev()
                .find(|hop| !self.trusts(**hop))
                .or(hops.first())
                .copied()
                .unwrap_or(peer),
        }
    }
}

/// `addr` or `addr/prefix`, with the prefix within the address family.
pub fn parse_range(text: &str) -> Option<(IpAddr, u8)> {
    let (addr, prefix) = match text.trim().split_once('/') {
        Some((addr, prefix)) => (addr, Some(prefix)),
        None => (text.trim(), None),
    };
    let addr: IpAddr = addr.parse().ok()?;
    let max = if addr.is_ipv4() { 32 } else { 128 };
    let prefix = match prefix {
        Some(p) => p.parse::<u8>().ok().filter(|p| *p <= max)?,
        None => max,
    };
    Some((addr.to_canonical(), prefix))
}

fn in_range(ip: IpAddr, net: IpAddr, prefix: u8) -> bool {
    let mask = |bits: u32| u128::MAX.checked_shl(bits - u32::from(prefix)).unwrap_or(0);
    match (ip, net) {
        (IpAddr::V4(ip), IpAddr::V4(net)) => {
            let mask = mask(32) & u128::from(u32::MAX);
            u128::from(u32::from(ip)) & mask == u128::from(u32::from(net)) & mask
        }
        (IpAddr::V6(ip), IpAddr::V6(net)) => {
            let mask = mask(128);
            u128::from(ip) & mask == u128::from(net) & mask
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    /// Bearer tokens required for protected calls (401 challenge otherwise).
    #[default]
    Oauth,
    /// Fallback: missing credentials become guest/unapproved; invalid ones still fail.
    NoLogin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    #[serde(default)]
    pub mode: AuthMode,
    /// Authorization server issuer URL (exact `iss`; metadata via RFC 8414 or OIDC discovery).
    pub issuer: Url,
    /// Dotted claim path holding the role names, e.g. `roles` or `entitlements.roles`.
    #[serde(default = "default_roles_claim")]
    pub roles_claim: String,
}

fn default_roles_claim() -> String {
    "roles".to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxConfig {
    /// Runtime opt-in; only honoured by binaries built with the `sandbox` feature.
    #[serde(default)]
    pub enabled: bool,
}

/// Passive strings for the mascot client; no server behavior depends on them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MascotConfig {
    pub name: String,
    #[serde(default)]
    pub persona: Option<String>,
}

/// Per-host upstream limits; defaults follow the frozen runtime defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamLimits {
    #[serde(default = "d_rps")]
    pub requests_per_second: u32,
    #[serde(default = "d_burst")]
    pub burst: u32,
    #[serde(default = "d_timeout")]
    pub timeout_ms: u64,
}

impl Default for UpstreamLimits {
    fn default() -> Self {
        Self {
            requests_per_second: d_rps(),
            burst: d_burst(),
            timeout_ms: d_timeout(),
        }
    }
}

const fn d_rps() -> u32 {
    crate::runtime::defaults::UPSTREAM_REQUESTS_PER_SECOND
}
const fn d_burst() -> u32 {
    crate::runtime::defaults::UPSTREAM_BURST
}
#[allow(clippy::cast_possible_truncation)]
const fn d_timeout() -> u64 {
    crate::runtime::defaults::UPSTREAM_TIMEOUT.as_millis() as u64
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibCalLocation {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibCalConfig {
    pub enabled: bool,
    pub base_url: Url,
    pub locations: Vec<LibCalLocation>,
    pub default_location: String,
    /// Sandbox-only booking simulation.
    #[serde(default)]
    pub booking_simulation: bool,
    #[serde(default)]
    pub limits: UpstreamLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BannerConfig {
    pub enabled: bool,
    /// Banner 9 Student Registration SSB base, e.g. `https://host/StudentRegistrationSsb/ssb`.
    pub base_url: Url,
    pub default_term: String,
    pub allowed_terms: Vec<String>,
    #[serde(default)]
    pub limits: UpstreamLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParserProfile {
    /// Parser profile name understood by the hours adapter.
    pub profile: String,
    /// Named CSS selectors for that profile.
    #[serde(default)]
    pub selectors: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub id: String,
    pub name: String,
    pub url: Url,
    pub parser: ParserProfile,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoursConfig {
    pub enabled: bool,
    pub libraries: Vec<Library>,
    pub default_library: String,
    #[serde(default)]
    pub limits: UpstreamLimits,
}

/// Daily operating window in school-local time; `end` may be after midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceWindow {
    pub start: NaiveTime,
    pub end: NaiveTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitProvider {
    pub id: String,
    /// Adapter kind, e.g. `septa_arrivals`, `gtfs_static`, `transloc_relay`.
    pub kind: String,
    #[serde(default)]
    pub base_url: Option<Url>,
    #[serde(default)]
    pub service_window: Option<ServiceWindow>,
    #[serde(default)]
    pub limits: UpstreamLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitStop {
    /// Canonical stop ID as the provider knows it.
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitRoute {
    pub id: String,
    pub provider: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GtfsValidity {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitConfig {
    pub enabled: bool,
    pub providers: Vec<TransitProvider>,
    pub stops: Vec<TransitStop>,
    #[serde(default)]
    pub routes: Vec<TransitRoute>,
    #[serde(default)]
    pub gtfs_subset_path: Option<PathBuf>,
    #[serde(default)]
    pub gtfs_validity: Option<GtfsValidity>,
    /// Recorded Flight replay; sandbox only.
    #[serde(default)]
    pub replay_path: Option<PathBuf>,
}

/// Sandbox-only synthetic Canvas-shaped fake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasConfig {
    pub enabled: bool,
    /// Loopback URL of the local fake.
    pub base_url: Url,
    /// Dedicated synthetic service credential for the fake.
    #[serde(default)]
    pub service_token_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSticker {
    /// Demo sticker UID (never logged).
    pub uid: String,
    /// Synthetic subject; must start with `synthetic:`.
    pub subject: String,
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceConfig {
    pub device_id: String,
    /// Client ID (`client_id`, or `azp`) of the device's MCP token.
    pub client_id: String,
    /// Env var holding the Pi-only bind secret.
    pub bind_secret_env: String,
    pub stickers: Vec<DeviceSticker>,
}

pub(super) fn validate(c: &SchoolConfig, env: &Env<'_>) -> Vec<super::ConfigIssue> {
    let mut v = Issues::default();
    if c.version != 1 {
        v.push("version", "must be 1");
    }
    v.id("school.id", &c.school.id);
    if c.school.display_name.trim().is_empty() || c.school.display_name.chars().count() > 160 {
        v.push("school.display_name", "must be 1-160 characters");
    }
    if c.school.timezone.parse::<chrono_tz::Tz>().is_err() {
        v.push("school.timezone", "unknown IANA timezone");
    }
    validate_server(&mut v, &c.server);
    v.url("auth.issuer", &c.auth.issuer);
    if c.auth
        .roles_claim
        .split('.')
        .any(|key| key.trim().is_empty())
    {
        v.push("auth.roles_claim", "must be a dotted claim path");
    }
    v.unique("modules", c.modules.iter().map(String::as_str));

    let sandbox_on = c.sandbox.enabled && env.sandbox_compiled;
    if c.sandbox.enabled && !env.sandbox_compiled {
        v.push(
            "sandbox.enabled",
            "this binary was built without the sandbox feature",
        );
    }
    let mut needs_sandbox = |field: &str, used: bool| {
        if used && !sandbox_on {
            v.push(field, "requires a sandbox build with sandbox.enabled: true");
        }
    };
    needs_sandbox(
        "libcal.booking_simulation",
        c.libcal.as_ref().is_some_and(|l| l.booking_simulation),
    );
    needs_sandbox(
        "transit.replay_path",
        c.transit.as_ref().is_some_and(|t| t.replay_path.is_some()),
    );
    needs_sandbox(
        "canvas.enabled",
        c.canvas.as_ref().is_some_and(|x| x.enabled),
    );

    if let Some(l) = &c.libcal {
        v.url("libcal.base_url", &l.base_url);
        for (i, loc) in l.locations.iter().enumerate() {
            v.id(&format!("libcal.locations[{i}].id"), &loc.id);
        }
        v.unique(
            "libcal.locations",
            l.locations.iter().map(|x| x.id.as_str()),
        );
        if !l.locations.iter().any(|x| x.id == l.default_location) {
            v.push("libcal.default_location", "must be one of libcal.locations");
        }
        limits(&mut v, "libcal.limits", l.limits);
    }
    if let Some(b) = &c.banner {
        v.url("banner.base_url", &b.base_url);
        v.unique(
            "banner.allowed_terms",
            b.allowed_terms.iter().map(String::as_str),
        );
        if !b.allowed_terms.contains(&b.default_term) {
            v.push("banner.default_term", "must be one of banner.allowed_terms");
        }
        limits(&mut v, "banner.limits", b.limits);
    }
    if let Some(h) = &c.hours {
        for (i, lib) in h.libraries.iter().enumerate() {
            v.id(&format!("hours.libraries[{i}].id"), &lib.id);
            v.url(&format!("hours.libraries[{i}].url"), &lib.url);
            if lib.parser.profile.trim().is_empty() {
                v.push(
                    format!("hours.libraries[{i}].parser.profile"),
                    "must not be empty",
                );
            }
        }
        v.unique("hours.libraries", h.libraries.iter().map(|x| x.id.as_str()));
        if !h.libraries.iter().any(|x| x.id == h.default_library) {
            v.push("hours.default_library", "must be one of hours.libraries");
        }
        limits(&mut v, "hours.limits", h.limits);
    }
    if let Some(t) = &c.transit {
        validate_transit(&mut v, t);
    }
    if let Some(cv) = &c.canvas {
        v.loopback_url("canvas.base_url", &cv.base_url);
        if let Some(name) = &cv.service_token_env {
            v.secret_env("canvas.service_token_env", name, env);
        }
    }
    validate_devices(&mut v, &c.devices, env);
    v.0
}

fn validate_devices(v: &mut Issues, devices: &[DeviceConfig], env: &Env<'_>) {
    v.unique("devices", devices.iter().map(|d| d.device_id.as_str()));
    for (i, d) in devices.iter().enumerate() {
        let f = format!("devices[{i}]");
        v.id(&format!("{f}.device_id"), &d.device_id);
        v.id(&format!("{f}.client_id"), &d.client_id);
        v.secret_env(&format!("{f}.bind_secret_env"), &d.bind_secret_env, env);
        v.unique(
            &format!("{f}.stickers"),
            d.stickers.iter().map(|s| s.uid.as_str()),
        );
        for (j, s) in d.stickers.iter().enumerate() {
            let sf = format!("{f}.stickers[{j}]");
            if s.uid.is_empty() || s.uid.len() > 128 {
                v.push(format!("{sf}.uid"), "must be 1-128 bytes");
            }
            if !s.subject.starts_with("synthetic:") {
                v.push(
                    format!("{sf}.subject"),
                    "must be a synthetic subject (synthetic:...)",
                );
            }
            if s.role == Role::Admin {
                v.push(format!("{sf}.role"), "stickers cannot grant admin");
            }
        }
    }
}

fn validate_server(v: &mut Issues, server: &ServerConfig) {
    v.url("server.resource_url", &server.resource_url);
    if !server.admin_listen.ip().is_loopback() {
        v.push("server.admin_listen", "must be a loopback address");
    }
    if server.admin_listen == server.listen {
        v.push("server.admin_listen", "must differ from server.listen");
    }
    if let Some(proxy) = &server.trusted_proxy {
        validate_proxy(v, proxy);
    }
}

fn validate_proxy(v: &mut Issues, proxy: &TrustedProxy) {
    if proxy.peers.is_empty() {
        v.push(
            "server.trusted_proxy.peers",
            "must list at least one proxy address",
        );
    }
    for (i, peer) in proxy.peers.iter().enumerate() {
        if parse_range(peer).is_none() {
            v.push(
                format!("server.trusted_proxy.peers[{i}]"),
                "must be an IP address or CIDR range",
            );
        }
    }
}

fn validate_transit(v: &mut Issues, t: &TransitConfig) {
    v.unique(
        "transit.providers",
        t.providers.iter().map(|p| p.id.as_str()),
    );
    for (i, p) in t.providers.iter().enumerate() {
        v.id(&format!("transit.providers[{i}].id"), &p.id);
        if let Some(url) = &p.base_url {
            v.url(&format!("transit.providers[{i}].base_url"), url);
        }
        limits(v, &format!("transit.providers[{i}].limits"), p.limits);
    }
    let providers: BTreeSet<&str> = t.providers.iter().map(|p| p.id.as_str()).collect();
    for (i, s) in t.stops.iter().enumerate() {
        if !providers.contains(s.provider.as_str()) {
            v.push(format!("transit.stops[{i}].provider"), "unknown provider");
        }
    }
    for (i, r) in t.routes.iter().enumerate() {
        if !providers.contains(r.provider.as_str()) {
            v.push(format!("transit.routes[{i}].provider"), "unknown provider");
        }
    }
    v.unique("transit.stops", t.stops.iter().map(|s| s.id.as_str()));
    v.unique("transit.routes", t.routes.iter().map(|r| r.id.as_str()));
    if let Some(g) = t.gtfs_validity
        && g.end < g.start
    {
        v.push("transit.gtfs_validity", "end is before start");
    }
}

fn limits(v: &mut Issues, field: &str, l: UpstreamLimits) {
    if l.requests_per_second == 0 || l.burst == 0 || l.timeout_ms == 0 || l.timeout_ms > 10_000 {
        v.push(
            field,
            "rates must be positive and timeout_ms within 1..=10000",
        );
    }
}
