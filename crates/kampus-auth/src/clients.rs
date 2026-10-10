//! The three client kinds: CIMD (an exact allowlist of client ID URLs;
//! public, PKCE only), pre-registered confidential clients and device
//! clients (`client_credentials` only). Secrets come from the environment
//! and are compared in constant time.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use kampus_core::config::BrokerConfig;
use kampus_core::contracts::Role;

use crate::error::OAuthError;
use crate::util::{b64_decode, form_unescape, secret_eq};

/// A secret that never prints.
#[derive(Clone)]
pub(crate) struct Secret(String);

impl Secret {
    pub(crate) fn matches(&self, presented: &str) -> bool {
        secret_eq(&self.0, presented)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Confidential {
    pub(crate) id: String,
    pub(crate) label: String,
    secret: Secret,
    pub(crate) redirect_uris: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Device {
    pub(crate) id: String,
    secret: Secret,
    pub(crate) roles: BTreeSet<Role>,
    pub(crate) scopes: BTreeSet<String>,
}

/// Every client the broker knows.
#[derive(Debug, Default)]
pub(crate) struct Clients {
    confidential: BTreeMap<String, Confidential>,
    devices: BTreeMap<String, Device>,
    cimd: BTreeSet<String>,
}

/// What a client ID refers to.
#[derive(Debug)]
pub(crate) enum Lookup<'a> {
    Confidential(&'a Confidential),
    Device(&'a Device),
    /// An allowlisted CIMD URL (the document still has to be fetched).
    Cimd(&'a str),
    Unknown,
}

/// A missing secret environment variable (named, never valued).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("environment variable {0} is missing or empty")]
pub struct MissingSecret(pub String);

impl Clients {
    pub(crate) fn from_config(
        config: &BrokerConfig,
        secret: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, MissingSecret> {
        let read = |name: &str| {
            secret(name)
                .filter(|v| !v.is_empty())
                .map(Secret)
                .ok_or_else(|| MissingSecret(name.to_owned()))
        };
        let mut clients = Self {
            cimd: config.cimd_clients.iter().cloned().collect(),
            ..Self::default()
        };
        for c in &config.clients {
            clients.confidential.insert(
                c.client_id.clone(),
                Confidential {
                    id: c.client_id.clone(),
                    label: c.label.clone(),
                    secret: read(&c.secret_env)?,
                    redirect_uris: c.redirect_uris.clone(),
                },
            );
        }
        for d in &config.devices {
            clients.devices.insert(
                d.client_id.clone(),
                Device {
                    id: d.client_id.clone(),
                    secret: read(&d.secret_env)?,
                    roles: d.roles.clone(),
                    scopes: d.scopes.clone(),
                },
            );
        }
        Ok(clients)
    }

    /// Configured clients first, then the exact CIMD allowlist. Anything
    /// else is unknown and never triggers a fetch.
    pub(crate) fn lookup(&self, client_id: &str) -> Lookup<'_> {
        if let Some(c) = self.confidential.get(client_id) {
            Lookup::Confidential(c)
        } else if let Some(d) = self.devices.get(client_id) {
            Lookup::Device(d)
        } else if let Some(url) = self.cimd.get(client_id) {
            Lookup::Cimd(url)
        } else {
            Lookup::Unknown
        }
    }

    /// Authenticate a token-endpoint client: CIMD clients are public (no
    /// secret allowed), configured clients need their secret by exactly one
    /// method.
    pub(crate) fn authenticate(&self, creds: &ClientCredentials) -> Result<Client, OAuthError> {
        let check = |secret: &Secret| match (&creds.method, &creds.secret) {
            (AuthMethod::Basic | AuthMethod::Post, Some(presented))
                if secret.matches(presented) =>
            {
                Ok(())
            }
            _ => Err(OAuthError::invalid_client(
                creds.method == AuthMethod::Basic,
            )),
        };
        match self.lookup(&creds.client_id) {
            Lookup::Confidential(c) => check(&c.secret).map(|()| Client::Confidential(c.clone())),
            Lookup::Device(d) => check(&d.secret).map(|()| Client::Device(d.clone())),
            Lookup::Cimd(url) if creds.method == AuthMethod::None => {
                Ok(Client::Cimd(url.to_owned()))
            }
            Lookup::Cimd(_) | Lookup::Unknown => Err(OAuthError::invalid_client(
                creds.method == AuthMethod::Basic,
            )),
        }
    }
}

/// An authenticated token-endpoint client.
#[derive(Debug, Clone)]
pub(crate) enum Client {
    Cimd(String),
    Confidential(Confidential),
    Device(Device),
}

impl Client {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Cimd(id) => id,
            Self::Confidential(c) => &c.id,
            Self::Device(d) => &d.id,
        }
    }
}

/// How the client authenticated at the token endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthMethod {
    /// Public client: `client_id` only.
    None,
    /// `client_secret_basic`.
    Basic,
    /// `client_secret_post`.
    Post,
}

/// Client identification presented at the token endpoint.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ClientCredentials {
    pub(crate) client_id: String,
    pub(crate) secret: Option<String>,
    pub(crate) method: AuthMethod,
}

impl fmt::Debug for ClientCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientCredentials")
            .field("client_id", &self.client_id)
            .field("method", &self.method)
            .finish_non_exhaustive()
    }
}

/// A parsed token request: form parameters (each at most once) plus the
/// client's credentials from exactly one method.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TokenRequest {
    pub(crate) grant_type: String,
    pub(crate) client: ClientCredentials,
    params: BTreeMap<String, String>,
}

impl fmt::Debug for TokenRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenRequest")
            .field("grant_type", &self.grant_type)
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

impl TokenRequest {
    pub(crate) fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(String::as_str)
    }
}

/// Largest token request body accepted.
pub(crate) const MAX_TOKEN_BODY: usize = 8 * 1024;

/// Parse a form-encoded token request body and the `Authorization` header.
/// Pure (fuzz target `token_form`): no I/O, never panics.
pub(crate) fn parse_token_request(
    body: &[u8],
    authorization: Option<&str>,
) -> Result<TokenRequest, OAuthError> {
    if body.len() > MAX_TOKEN_BODY {
        return Err(OAuthError::invalid_request("request too large"));
    }
    let mut params = BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(body) {
        if params
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(OAuthError::invalid_request("repeated parameter"));
        }
    }
    let grant_type = params
        .remove("grant_type")
        .ok_or_else(|| OAuthError::invalid_request("grant_type is required"))?;
    let body_id = params.remove("client_id");
    let body_secret = params.remove("client_secret");
    let client = if let Some(header) = authorization {
        let (id, secret) = parse_basic(header)?;
        if body_secret.is_some() {
            return Err(OAuthError::invalid_request(
                "use exactly one client authentication method",
            ));
        }
        if body_id.as_deref().is_some_and(|b| b != id) {
            return Err(OAuthError::invalid_request("conflicting client_id"));
        }
        ClientCredentials {
            client_id: id,
            secret: Some(secret),
            method: AuthMethod::Basic,
        }
    } else {
        let client_id = body_id
            .filter(|id| !id.is_empty())
            .ok_or_else(|| OAuthError::invalid_request("client_id is required"))?;
        let method = if body_secret.is_some() {
            AuthMethod::Post
        } else {
            AuthMethod::None
        };
        ClientCredentials {
            client_id,
            secret: body_secret,
            method,
        }
    };
    Ok(TokenRequest {
        grant_type,
        client,
        params,
    })
}

/// RFC 6749 section 2.3.1: `Basic base64(urlencode(id) ":" urlencode(secret))`.
fn parse_basic(header: &str) -> Result<(String, String), OAuthError> {
    let malformed = || OAuthError::invalid_client(true);
    let (scheme, encoded) = header.trim().split_once(' ').ok_or_else(malformed)?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return Err(OAuthError::invalid_request(
            "unsupported Authorization scheme",
        ));
    }
    let decoded = b64_decode(encoded.trim()).ok_or_else(malformed)?;
    let decoded = String::from_utf8(decoded).map_err(|_| malformed())?;
    let (id, secret) = decoded.split_once(':').ok_or_else(malformed)?;
    match (form_unescape(id), form_unescape(secret)) {
        (Some(id), Some(secret)) if !id.is_empty() => Ok((id, secret)),
        _ => Err(malformed()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::b64url;

    fn clients() -> Clients {
        let config: BrokerConfig = serde_saphyr::from_str(
            r"
listen: 127.0.0.1:8082
key_file: k.json
subject_key_env: SUBJECT
cimd_clients: [https://claude.ai/oauth/mcp-oauth-client-metadata]
clients:
  - { client_id: kampus-claude, label: Claude, secret_env: CLAUDE, redirect_uris: [https://claude.ai/api/mcp/auth_callback] }
devices:
  - { client_id: kampus-mascot, secret_env: MASCOT, roles: [guest] }
",
        )
        .unwrap();
        Clients::from_config(&config, &|name| match name {
            "CLAUDE" => Some("s3cret".to_owned()),
            "MASCOT" => Some("m4scot".to_owned()),
            _ => None,
        })
        .unwrap()
    }

    fn basic(id: &str, secret: &str) -> String {
        format!("Basic {}", b64url(format!("{id}:{secret}").as_bytes()))
    }

    fn auth(body: &str, header: Option<&str>) -> Result<Client, OAuthError> {
        let request = parse_token_request(body.as_bytes(), header)?;
        clients().authenticate(&request.client)
    }

    #[test]
    fn each_method_authenticates() {
        let ok = auth("grant_type=x", Some(&basic("kampus-claude", "s3cret"))).unwrap();
        assert_eq!(ok.id(), "kampus-claude");
        let ok = auth(
            "grant_type=x&client_id=kampus-claude&client_secret=s3cret",
            None,
        )
        .unwrap();
        assert!(matches!(ok, Client::Confidential(_)));
        let ok = auth(
            "grant_type=x&client_id=https%3A%2F%2Fclaude.ai%2Foauth%2Fmcp-oauth-client-metadata",
            None,
        )
        .unwrap();
        assert!(matches!(ok, Client::Cimd(_)));
        let ok = auth(
            "grant_type=x&client_id=kampus-mascot&client_secret=m4scot",
            None,
        )
        .unwrap();
        assert!(matches!(ok, Client::Device(_)));
    }

    #[test]
    fn wrong_missing_or_doubled_credentials_fail() {
        let err = auth("grant_type=x", Some(&basic("kampus-claude", "nope"))).unwrap_err();
        assert_eq!((err.code, err.status), ("invalid_client", 401));
        assert!(err.basic_challenge);
        let err = auth("grant_type=x&client_id=kampus-claude", None).unwrap_err();
        assert_eq!((err.code, err.status), ("invalid_client", 401));
        let err = auth(
            "grant_type=x&client_secret=s3cret",
            Some(&basic("kampus-claude", "s3cret")),
        )
        .unwrap_err();
        assert_eq!((err.code, err.status), ("invalid_request", 400));
        let err = auth("grant_type=x&client_id=someone", None).unwrap_err();
        assert_eq!((err.code, err.status), ("invalid_client", 401));
        // A public CIMD client may not present a secret.
        let err = auth(
            "grant_type=x&client_id=https%3A%2F%2Fclaude.ai%2Foauth%2Fmcp-oauth-client-metadata&client_secret=x",
            None,
        )
        .unwrap_err();
        assert_eq!(err.code, "invalid_client");
    }

    #[test]
    fn form_rules() {
        let err = parse_token_request(b"grant_type=a&grant_type=b&client_id=x", None).unwrap_err();
        assert_eq!(err.code, "invalid_request");
        let err = parse_token_request(b"client_id=x", None).unwrap_err();
        assert_eq!(err.code, "invalid_request");
        let err = parse_token_request(b"grant_type=x&client_id=x", Some("Bearer abc")).unwrap_err();
        assert_eq!(err.code, "invalid_request");
        let err = parse_token_request(
            b"grant_type=x&client_id=other",
            Some(&basic("kampus-claude", "s")),
        )
        .unwrap_err();
        assert_eq!(err.code, "invalid_request");
        // Form-encoded Basic credentials are decoded.
        let r = parse_token_request(b"grant_type=x", Some(&basic("a%3Ab", "p%25"))).unwrap();
        assert_eq!(r.client.client_id, "a:b");
        assert_eq!(r.client.secret.as_deref(), Some("p%"));
        assert!(!format!("{r:?}").contains("p%"));
    }
}
