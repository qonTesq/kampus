//! The signing-key file: an active Ed25519 key and at most one previous key,
//! each identified by its RFC 7638 thumbprint. `serve` creates the file when
//! it is absent; `check` never touches it. Rotation writes a temporary file
//! in the same directory and renames it over the original.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use aws_lc_rs::digest;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::util::{b64_decode, b64url, unix_now};

/// Why the key file could not be used. Never contains key material.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("cannot read the key file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot write the key file {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the key file {path} is malformed or holds a non-Ed25519 key")]
    Invalid { path: PathBuf },
    #[error("cannot generate a signing key")]
    Generate,
    #[error("cannot sign an access token")]
    Sign,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFile {
    active: StoredKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous: Option<StoredKey>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredKey {
    kid: String,
    /// Unix seconds.
    created: i64,
    /// PKCS#8 v2 document, base64.
    pkcs8_b64: String,
}

/// One usable key.
struct Key {
    kid: String,
    public: Vec<u8>,
    encoding: EncodingKey,
}

impl Key {
    fn from_stored(stored: &StoredKey) -> Option<Self> {
        let pkcs8 = b64_decode(&stored.pkcs8_b64)?;
        // Refuses anything that is not an Ed25519 PKCS#8 document.
        let pair = Ed25519KeyPair::from_pkcs8(&pkcs8).ok()?;
        let public = pair.public_key().as_ref().to_vec();
        if thumbprint(&public) != stored.kid {
            return None;
        }
        Some(Self {
            kid: stored.kid.clone(),
            public,
            encoding: EncodingKey::from_ed_der(&pkcs8),
        })
    }

    fn jwk(&self) -> Value {
        json!({
            "kty": "OKP", "crv": "Ed25519", "use": "sig", "alg": "EdDSA",
            "kid": self.kid, "x": b64url(&self.public),
        })
    }
}

/// The loaded signing keys.
pub struct SigningKeys {
    active: Key,
    previous: Option<Key>,
}

impl std::fmt::Debug for SigningKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigningKeys")
            .field("active", &self.active.kid)
            .field("previous", &self.previous.as_ref().map(|k| &k.kid))
            .finish()
    }
}

impl SigningKeys {
    /// Load the key file, creating it (owner-only) when it does not exist.
    /// An existing file that cannot be used is never overwritten.
    pub fn load_or_create(path: &Path) -> Result<Self, KeyError> {
        match fs::read(path) {
            Ok(bytes) => Self::parse(path, &bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let file = KeyFile {
                    active: generate()?,
                    previous: None,
                };
                write_new(path, &file)?;
                Self::from_file(path, &file)
            }
            Err(source) => Err(KeyError::Read {
                path: path.to_owned(),
                source,
            }),
        }
    }

    /// Load an existing key file without creating anything.
    pub fn load(path: &Path) -> Result<Self, KeyError> {
        let bytes = fs::read(path).map_err(|source| KeyError::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::parse(path, &bytes)
    }

    fn parse(path: &Path, bytes: &[u8]) -> Result<Self, KeyError> {
        let file: KeyFile = serde_json::from_slice(bytes).map_err(|_| KeyError::Invalid {
            path: path.to_owned(),
        })?;
        Self::from_file(path, &file)
    }

    fn from_file(path: &Path, file: &KeyFile) -> Result<Self, KeyError> {
        let invalid = || KeyError::Invalid {
            path: path.to_owned(),
        };
        let active = Key::from_stored(&file.active).ok_or_else(invalid)?;
        let previous = match &file.previous {
            Some(stored) => Some(Key::from_stored(stored).ok_or_else(invalid)?),
            None => None,
        };
        if previous.as_ref().is_some_and(|p| p.kid == active.kid) {
            return Err(invalid());
        }
        Ok(Self { active, previous })
    }

    /// The active key's ID.
    pub fn active_kid(&self) -> &str {
        &self.active.kid
    }

    /// Public JWKS: the active key and the previous one, if any.
    pub fn jwks(&self) -> Value {
        let keys: Vec<Value> = std::iter::once(&self.active)
            .chain(self.previous.as_ref())
            .map(Key::jwk)
            .collect();
        json!({ "keys": keys })
    }

    /// Sign RFC 9068 claims: header `{alg: EdDSA, typ: at+jwt, kid}`.
    pub fn sign(&self, claims: &impl Serialize) -> Result<String, KeyError> {
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some("at+jwt".to_owned());
        header.kid = Some(self.active.kid.clone());
        jsonwebtoken::encode(&header, claims, &self.active.encoding).map_err(|_| KeyError::Sign)
    }
}

/// Generate a new active key, demote the active key to previous and drop
/// the older previous key. The file is replaced atomically; a failed write
/// leaves the old file untouched. Takes effect when kampus restarts.
pub fn rotate_key(path: &Path) -> Result<(), KeyError> {
    let bytes = fs::read(path).map_err(|source| KeyError::Read {
        path: path.to_owned(),
        source,
    })?;
    let current: KeyFile = serde_json::from_slice(&bytes).map_err(|_| KeyError::Invalid {
        path: path.to_owned(),
    })?;
    // Refuse to rotate a file that would not load.
    SigningKeys::from_file(path, &current)?;
    let next = KeyFile {
        active: generate()?,
        previous: Some(current.active),
    };
    replace(path, &next)
}

fn generate() -> Result<StoredKey, KeyError> {
    let pkcs8 =
        Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| KeyError::Generate)?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_| KeyError::Generate)?;
    Ok(StoredKey {
        kid: thumbprint(pair.public_key().as_ref()),
        created: unix_now(),
        pkcs8_b64: b64url(pkcs8.as_ref()),
    })
}

/// RFC 7638 JWK thumbprint of an Ed25519 public key (RFC 8037 members).
fn thumbprint(public: &[u8]) -> String {
    let canonical = format!(
        r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#,
        b64url(public)
    );
    b64url(digest::digest(&digest::SHA256, canonical.as_bytes()).as_ref())
}

fn serialize(path: &Path, file: &KeyFile) -> Result<Vec<u8>, KeyError> {
    serde_json::to_vec_pretty(file).map_err(|e| KeyError::Write {
        path: path.to_owned(),
        source: std::io::Error::other(e),
    })
}

/// Create `path` with owner-only permissions (Unix); refuses to replace an
/// existing file.
fn write_new(path: &Path, file: &KeyFile) -> Result<(), KeyError> {
    let bytes = serialize(path, file)?;
    let write_err = |source| KeyError::Write {
        path: path.to_owned(),
        source,
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(write_err)?;
    }
    let mut out = owner_only()
        .create_new(true)
        .open(path)
        .map_err(write_err)?;
    out.write_all(&bytes).map_err(write_err)?;
    out.sync_all().map_err(write_err)
}

/// Write to a temporary file in the same directory, then rename it over `path`.
fn replace(path: &Path, file: &KeyFile) -> Result<(), KeyError> {
    let bytes = serialize(path, file)?;
    let write_err = |source| KeyError::Write {
        path: path.to_owned(),
        source,
    };
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let written = (|| {
        let mut out = owner_only().create(true).truncate(true).open(&tmp)?;
        out.write_all(&bytes)?;
        out.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written.map_err(write_err)
}

fn owner_only() -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kampus-auth-keys-{name}-{}",
            crate::util::random_hex(6).unwrap()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn kids(keys: &SigningKeys) -> Vec<String> {
        keys.jwks()["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["kid"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn creates_then_reloads_the_same_key() {
        let dir = temp_dir("create");
        let path = dir.join("nested/keys.json");
        let created = SigningKeys::load_or_create(&path).unwrap();
        let reloaded = SigningKeys::load_or_create(&path).unwrap();
        assert_eq!(created.active_kid(), reloaded.active_kid());
        assert_eq!(kids(&created), [created.active_kid()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn jwks_holds_public_parts_only() {
        let dir = temp_dir("public");
        let keys = SigningKeys::load_or_create(&dir.join("k.json")).unwrap();
        let jwk = &keys.jwks()["keys"][0];
        let mut members: Vec<&str> = jwk
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        members.sort_unstable();
        assert_eq!(members, ["alg", "crv", "kid", "kty", "use", "x"]);
        assert_eq!(b64_decode(jwk["x"].as_str().unwrap()).unwrap().len(), 32);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_or_foreign_files_fail_and_stay_untouched() {
        let dir = temp_dir("corrupt");
        let path = dir.join("k.json");
        let p256 = aws_lc_rs::signature::EcdsaKeyPair::generate_pkcs8(
            &aws_lc_rs::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &SystemRandom::new(),
        )
        .unwrap();
        let foreign =
            json!({ "active": { "kid": "x", "created": 0, "pkcs8_b64": b64url(p256.as_ref()) } });
        for body in [
            b"not json".to_vec(),
            br#"{"active":{}}"#.to_vec(),
            serde_json::to_vec(&foreign).unwrap(),
        ] {
            fs::write(&path, &body).unwrap();
            assert!(matches!(
                SigningKeys::load_or_create(&path),
                Err(KeyError::Invalid { .. })
            ));
            assert!(rotate_key(&path).is_err());
            assert_eq!(fs::read(&path).unwrap(), body);
        }
        // A tampered kid is refused too.
        fs::remove_file(&path).unwrap();
        SigningKeys::load_or_create(&path).unwrap();
        let mut file: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        file["active"]["kid"] = json!("someone-else");
        fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        assert!(SigningKeys::load(&path).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rotation_demotes_then_drops() {
        let dir = temp_dir("rotate");
        let path = dir.join("k.json");
        let first = SigningKeys::load_or_create(&path).unwrap();
        let k1 = first.active_kid().to_owned();
        rotate_key(&path).unwrap();
        let second = SigningKeys::load(&path).unwrap();
        let k2 = second.active_kid().to_owned();
        assert_ne!(k1, k2);
        assert_eq!(kids(&second), [k2.clone(), k1.clone()]);
        rotate_key(&path).unwrap();
        let third = SigningKeys::load(&path).unwrap();
        assert_eq!(kids(&third)[1], k2);
        assert!(!kids(&third).contains(&k1));
        assert!(!dir.join("k.json.tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_rotation_leaves_the_old_file() {
        let dir = temp_dir("rotate-fail");
        let path = dir.join("k.json");
        SigningKeys::load_or_create(&path).unwrap();
        let before = fs::read(&path).unwrap();
        // A directory where the temporary file must go makes the write fail.
        fs::create_dir(dir.join("k.json.tmp")).unwrap();
        assert!(matches!(rotate_key(&path), Err(KeyError::Write { .. })));
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn signed_tokens_carry_the_profile_header() {
        let dir = temp_dir("sign");
        let keys = SigningKeys::load_or_create(&dir.join("k.json")).unwrap();
        let token = keys.sign(&json!({ "sub": "x" })).unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.alg, Algorithm::EdDSA);
        assert_eq!(header.typ.as_deref(), Some("at+jwt"));
        assert_eq!(header.kid.as_deref(), Some(keys.active_kid()));
        fs::remove_dir_all(dir).unwrap();
    }
}
