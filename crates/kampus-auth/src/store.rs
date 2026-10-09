//! In-memory broker state behind a small trait (the seam for a persistent
//! store; `MemoryStore` is the only implementation). Every collection has a
//! TTL and a fixed capacity. When one is full, new unauthenticated work is
//! refused; live grants and refresh families are never evicted. Locks are
//! short and never held across I/O.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use kampus_core::contracts::Role;

/// Fixed capacities (design §11).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Capacity {
    pub(crate) pending: usize,
    #[allow(dead_code, reason = "upstream OIDC login state arrives in group 6")]
    pub(crate) upstream: usize,
    #[cfg_attr(
        not(feature = "sandbox"),
        allow(
            dead_code,
            reason = "without sandbox demo login, sessions are created by upstream login (group 6)"
        )
    )]
    pub(crate) sessions: usize,
    pub(crate) codes: usize,
    pub(crate) grants: usize,
    pub(crate) families: usize,
}

impl Capacity {
    pub(crate) const DEFAULT: Self = Self {
        pending: 10_000,
        upstream: 10_000,
        sessions: 10_000,
        codes: 20_000,
        grants: 50_000,
        families: 50_000,
    };
}

/// A collection is at capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Full;

/// A validated authorization request waiting for sign-in and consent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pending {
    pub(crate) client_id: String,
    /// The CIMD client ID's host or the configured label; never a self-asserted name.
    pub(crate) client_label: String,
    pub(crate) redirect_uri: String,
    pub(crate) state: Option<String>,
    pub(crate) code_challenge: String,
    /// Grantable scopes (without `offline_access`).
    pub(crate) scopes: BTreeSet<String>,
    /// Consent-form CSRF token bound to this request.
    pub(crate) csrf: String,
    pub(crate) expires_at: i64,
}

/// Upstream OIDC login state, keyed by its own `state`.
#[allow(dead_code, reason = "upstream OIDC login arrives in group 6")]
#[derive(Clone)]
pub(crate) struct UpstreamLogin {
    pub(crate) pending: String,
    pub(crate) nonce: String,
    pub(crate) pkce_verifier: String,
    pub(crate) expires_at: i64,
}

impl fmt::Debug for UpstreamLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpstreamLogin").finish_non_exhaustive()
    }
}

/// A signed-in browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Session {
    pub(crate) subject: String,
    pub(crate) roles: BTreeSet<Role>,
    /// Shown on pages ("Student A (synthetic demo user)"); never put in tokens.
    pub(crate) label: String,
    /// CSRF token for the connected-assistants page.
    pub(crate) csrf: String,
    pub(crate) expires_at: i64,
}

/// One grant: consent, refresh and revocation share this key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct GrantKey {
    pub(crate) subject: String,
    pub(crate) client_id: String,
    pub(crate) resource: String,
    pub(crate) scopes: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Grant {
    pub(crate) id: String,
    pub(crate) key: GrantKey,
    pub(crate) label: String,
    pub(crate) granted_at: i64,
}

/// An issued authorization code (stored under its SHA-256).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Code {
    pub(crate) grant: GrantKey,
    pub(crate) redirect_uri: String,
    /// The S256 PKCE challenge.
    pub(crate) challenge: String,
    pub(crate) roles: BTreeSet<Role>,
    pub(crate) expires_at: i64,
}

#[derive(Debug, Clone)]
enum CodeSlot {
    Live(Code),
    /// Redeemed; kept until the original expiry plus 10 minutes so reuse
    /// can revoke what the first redemption issued.
    Used {
        family: Option<String>,
        until: i64,
    },
}

/// The outcome of presenting a code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Redemption {
    Fresh(Code),
    /// Already redeemed: the family it created (now revoked by the caller).
    Reused(Option<String>),
    Unknown,
}

/// A refresh-token family: one grant, one current token hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Family {
    pub(crate) id: String,
    pub(crate) grant: GrantKey,
    pub(crate) roles: BTreeSet<Role>,
    pub(crate) current: String,
    pub(crate) expires_at: i64,
}

/// The outcome of presenting a refresh token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Rotation {
    Rotated(Family),
    /// A retired token: the family is now revoked.
    Reused,
    Invalid,
}

/// Retired hashes remembered per family for reuse detection.
const MAX_RETIRED: usize = 2048;
/// How long a redeemed code's tombstone outlives the code.
const TOMBSTONE_SECONDS: i64 = 600;

/// Broker state.
pub(crate) trait Store: Send + Sync + fmt::Debug {
    fn insert_pending(&self, id: String, pending: Pending) -> Result<(), Full>;
    fn pending(&self, id: &str, now: i64) -> Option<Pending>;
    fn take_pending(&self, id: &str, now: i64) -> Option<Pending>;

    #[allow(dead_code, reason = "upstream OIDC login arrives in group 6")]
    fn insert_upstream(&self, state: String, login: UpstreamLogin) -> Result<(), Full>;
    #[allow(dead_code, reason = "upstream OIDC login arrives in group 6")]
    fn take_upstream(&self, state: &str, now: i64) -> Option<UpstreamLogin>;

    fn insert_session(&self, id: String, session: Session) -> Result<(), Full>;
    fn session(&self, id: &str, now: i64) -> Option<Session>;

    fn insert_code(&self, hash: String, code: Code) -> Result<(), Full>;
    /// Single use: a live code becomes a tombstone.
    fn redeem_code(&self, hash: &str, now: i64) -> Redemption;
    /// Record the family a redemption created, for reuse revocation.
    fn bind_code_family(&self, hash: &str, family: &str);

    /// Insert the grant for `key`, or return the existing one.
    fn upsert_grant(&self, key: GrantKey, label: String, now: i64) -> Result<Grant, Full>;
    fn grant(&self, key: &GrantKey) -> Option<Grant>;
    fn grants_for(&self, subject: &str) -> Vec<Grant>;
    /// Delete a grant of `subject` and all its families; `None` on a miss.
    fn revoke_grant(&self, subject: &str, grant_id: &str) -> Option<Grant>;

    fn insert_family(&self, family: Family) -> Result<(), Full>;
    /// The live family whose current token hash is `hash`.
    fn family_by_token(&self, hash: &str, now: i64) -> Option<Family>;
    /// Rotate `presented` to `next`; a retired token revokes the family.
    fn rotate(&self, presented: &str, next: String, now: i64) -> Rotation;
    fn revoke_family(&self, family_id: &str);
    /// If `hash` is a retired token, revoke its family and return true.
    fn revoke_if_retired(&self, hash: &str) -> bool;

    fn purge(&self, now: i64);

    /// Pending authorizations held (tests check memory stays bounded).
    #[cfg(test)]
    fn pending_count(&self) -> usize;
}

#[derive(Debug, Default)]
struct Grants {
    by_id: HashMap<String, Grant>,
    by_key: HashMap<GrantKey, String>,
    families: HashMap<String, Family>,
    /// Current and retired token hashes -> family ID.
    tokens: HashMap<String, (String, bool)>,
    retired: HashMap<String, Vec<String>>,
}

impl Grants {
    fn drop_family(&mut self, id: &str) {
        if let Some(family) = self.families.remove(id) {
            self.tokens.remove(&family.current);
        }
        for hash in self.retired.remove(id).unwrap_or_default() {
            self.tokens.remove(&hash);
        }
    }
}

/// The in-memory store.
#[derive(Debug)]
pub(crate) struct MemoryStore {
    capacity: Capacity,
    pending: Mutex<HashMap<String, Pending>>,
    upstream: Mutex<HashMap<String, UpstreamLogin>>,
    sessions: Mutex<HashMap<String, Session>>,
    codes: Mutex<HashMap<String, CodeSlot>>,
    grants: Mutex<Grants>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn insert_bounded<V>(
    map: &mut HashMap<String, V>,
    cap: usize,
    key: String,
    value: V,
) -> Result<(), Full> {
    if map.len() >= cap && !map.contains_key(&key) {
        return Err(Full);
    }
    map.insert(key, value);
    Ok(())
}

impl MemoryStore {
    pub(crate) fn new(capacity: Capacity) -> Self {
        Self {
            capacity,
            pending: Mutex::default(),
            upstream: Mutex::default(),
            sessions: Mutex::default(),
            codes: Mutex::default(),
            grants: Mutex::default(),
        }
    }
}

impl Store for MemoryStore {
    fn insert_pending(&self, id: String, pending: Pending) -> Result<(), Full> {
        insert_bounded(&mut lock(&self.pending), self.capacity.pending, id, pending)
    }

    fn pending(&self, id: &str, now: i64) -> Option<Pending> {
        lock(&self.pending)
            .get(id)
            .filter(|p| p.expires_at > now)
            .cloned()
    }

    fn take_pending(&self, id: &str, now: i64) -> Option<Pending> {
        lock(&self.pending)
            .remove(id)
            .filter(|p| p.expires_at > now)
    }

    fn insert_upstream(&self, state: String, login: UpstreamLogin) -> Result<(), Full> {
        insert_bounded(
            &mut lock(&self.upstream),
            self.capacity.upstream,
            state,
            login,
        )
    }

    fn take_upstream(&self, state: &str, now: i64) -> Option<UpstreamLogin> {
        lock(&self.upstream)
            .remove(state)
            .filter(|u| u.expires_at > now)
    }

    fn insert_session(&self, id: String, session: Session) -> Result<(), Full> {
        insert_bounded(
            &mut lock(&self.sessions),
            self.capacity.sessions,
            id,
            session,
        )
    }

    fn session(&self, id: &str, now: i64) -> Option<Session> {
        lock(&self.sessions)
            .get(id)
            .filter(|s| s.expires_at > now)
            .cloned()
    }

    fn insert_code(&self, hash: String, code: Code) -> Result<(), Full> {
        insert_bounded(
            &mut lock(&self.codes),
            self.capacity.codes,
            hash,
            CodeSlot::Live(code),
        )
    }

    fn redeem_code(&self, hash: &str, now: i64) -> Redemption {
        let mut codes = lock(&self.codes);
        match codes.get(hash).cloned() {
            Some(CodeSlot::Live(code)) => {
                codes.insert(
                    hash.to_owned(),
                    CodeSlot::Used {
                        family: None,
                        until: code.expires_at + TOMBSTONE_SECONDS,
                    },
                );
                if code.expires_at > now {
                    Redemption::Fresh(code)
                } else {
                    Redemption::Unknown
                }
            }
            Some(CodeSlot::Used { family, .. }) => Redemption::Reused(family),
            None => Redemption::Unknown,
        }
    }

    fn bind_code_family(&self, hash: &str, family: &str) {
        if let Some(CodeSlot::Used { family: slot, .. }) = lock(&self.codes).get_mut(hash) {
            *slot = Some(family.to_owned());
        }
    }

    fn upsert_grant(&self, key: GrantKey, label: String, now: i64) -> Result<Grant, Full> {
        let mut g = lock(&self.grants);
        if let Some(existing) = g.by_key.get(&key).and_then(|id| g.by_id.get(id)) {
            return Ok(existing.clone());
        }
        if g.by_id.len() >= self.capacity.grants {
            return Err(Full);
        }
        let id = crate::util::random_hex(16).ok_or(Full)?;
        let grant = Grant {
            id: id.clone(),
            key: key.clone(),
            label,
            granted_at: now,
        };
        g.by_key.insert(key, id.clone());
        g.by_id.insert(id, grant.clone());
        Ok(grant)
    }

    fn grant(&self, key: &GrantKey) -> Option<Grant> {
        let g = lock(&self.grants);
        g.by_key.get(key).and_then(|id| g.by_id.get(id)).cloned()
    }

    fn grants_for(&self, subject: &str) -> Vec<Grant> {
        let mut grants: Vec<Grant> = lock(&self.grants)
            .by_id
            .values()
            .filter(|g| g.key.subject == subject)
            .cloned()
            .collect();
        grants.sort_by(|a, b| (a.granted_at, &a.id).cmp(&(b.granted_at, &b.id)));
        grants
    }

    fn revoke_grant(&self, subject: &str, grant_id: &str) -> Option<Grant> {
        let mut g = lock(&self.grants);
        let grant = g
            .by_id
            .get(grant_id)
            .filter(|grant| grant.key.subject == subject)
            .cloned()?;
        g.by_id.remove(grant_id);
        g.by_key.remove(&grant.key);
        let families: Vec<String> = g
            .families
            .values()
            .filter(|f| f.grant == grant.key)
            .map(|f| f.id.clone())
            .collect();
        for id in families {
            g.drop_family(&id);
        }
        Some(grant)
    }

    fn insert_family(&self, family: Family) -> Result<(), Full> {
        let mut g = lock(&self.grants);
        if g.families.len() >= self.capacity.families {
            return Err(Full);
        }
        g.tokens
            .insert(family.current.clone(), (family.id.clone(), true));
        g.families.insert(family.id.clone(), family);
        Ok(())
    }

    fn family_by_token(&self, hash: &str, now: i64) -> Option<Family> {
        let g = lock(&self.grants);
        match g.tokens.get(hash) {
            Some((id, true)) => g
                .families
                .get(id)
                .filter(|f| f.expires_at > now && g.by_key.contains_key(&f.grant))
                .cloned(),
            _ => None,
        }
    }

    fn rotate(&self, presented: &str, next: String, now: i64) -> Rotation {
        let mut g = lock(&self.grants);
        let Some((id, current)) = g.tokens.get(presented).cloned() else {
            return Rotation::Invalid;
        };
        if !current {
            // RFC 9700 section 4.14.2: reuse of a rotated token.
            g.drop_family(&id);
            return Rotation::Reused;
        }
        let live = g
            .families
            .get(&id)
            .is_some_and(|f| f.expires_at > now && g.by_key.contains_key(&f.grant));
        if !live {
            g.drop_family(&id);
            return Rotation::Invalid;
        }
        g.tokens.insert(presented.to_owned(), (id.clone(), false));
        g.tokens.insert(next.clone(), (id.clone(), true));
        let retired = g.retired.entry(id.clone()).or_default();
        retired.push(presented.to_owned());
        let overflow = retired.len().saturating_sub(MAX_RETIRED);
        let dropped: Vec<String> = retired.drain(..overflow).collect();
        for hash in dropped {
            g.tokens.remove(&hash);
        }
        match g.families.get_mut(&id) {
            Some(family) => {
                family.current = next;
                Rotation::Rotated(family.clone())
            }
            None => Rotation::Invalid,
        }
    }

    fn revoke_family(&self, family_id: &str) {
        lock(&self.grants).drop_family(family_id);
    }

    fn revoke_if_retired(&self, hash: &str) -> bool {
        let mut g = lock(&self.grants);
        match g.tokens.get(hash).cloned() {
            Some((id, false)) => {
                g.drop_family(&id);
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    fn pending_count(&self) -> usize {
        lock(&self.pending).len()
    }

    fn purge(&self, now: i64) {
        lock(&self.pending).retain(|_, p| p.expires_at > now);
        lock(&self.upstream).retain(|_, u| u.expires_at > now);
        lock(&self.sessions).retain(|_, s| s.expires_at > now);
        lock(&self.codes).retain(|_, slot| match slot {
            CodeSlot::Live(code) => code.expires_at + TOMBSTONE_SECONDS > now,
            CodeSlot::Used { until, .. } => *until > now,
        });
        let mut g = lock(&self.grants);
        let expired: Vec<String> = g
            .families
            .values()
            .filter(|f| f.expires_at <= now)
            .map(|f| f.id.clone())
            .collect();
        for id in expired {
            g.drop_family(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(subject: &str, scopes: &[&str]) -> GrantKey {
        GrantKey {
            subject: subject.to_owned(),
            client_id: "c".to_owned(),
            resource: "r".to_owned(),
            scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn pending(expires_at: i64) -> Pending {
        Pending {
            client_id: "c".into(),
            client_label: "c".into(),
            redirect_uri: "https://c/cb".into(),
            state: None,
            code_challenge: "x".into(),
            scopes: BTreeSet::new(),
            csrf: "t".into(),
            expires_at,
        }
    }

    fn family(id: &str, grant: GrantKey, current: &str) -> Family {
        Family {
            id: id.into(),
            grant,
            roles: BTreeSet::new(),
            current: current.into(),
            expires_at: 1000,
        }
    }

    #[test]
    fn entries_expire_and_purge() {
        let store = MemoryStore::new(Capacity::DEFAULT);
        store.insert_pending("p".into(), pending(100)).unwrap();
        assert!(store.pending("p", 99).is_some());
        assert!(store.pending("p", 100).is_none());
        store.purge(100);
        assert!(store.take_pending("p", 0).is_none());
    }

    #[test]
    fn full_collections_refuse_without_evicting_grants() {
        let store = MemoryStore::new(Capacity {
            pending: 2,
            grants: 1,
            ..Capacity::DEFAULT
        });
        store.insert_pending("a".into(), pending(100)).unwrap();
        store.insert_pending("b".into(), pending(100)).unwrap();
        assert_eq!(store.insert_pending("c".into(), pending(100)), Err(Full));
        let grant = store.upsert_grant(key("s", &[]), "l".into(), 0).unwrap();
        assert_eq!(store.upsert_grant(key("t", &[]), "l".into(), 0), Err(Full));
        // The existing grant is untouched and still found.
        assert_eq!(
            store.upsert_grant(key("s", &[]), "x".into(), 5).unwrap(),
            grant
        );
        assert_eq!(store.grants_for("s"), [grant]);
    }

    #[test]
    fn codes_are_single_use_with_a_tombstone() {
        let store = MemoryStore::new(Capacity::DEFAULT);
        let code = Code {
            grant: key("s", &[]),
            redirect_uri: "https://c/cb".into(),
            challenge: "x".into(),
            roles: BTreeSet::new(),
            expires_at: 60,
        };
        store.insert_code("h".into(), code.clone()).unwrap();
        assert_eq!(store.redeem_code("h", 1), Redemption::Fresh(code));
        store.bind_code_family("h", "f1");
        assert_eq!(
            store.redeem_code("h", 2),
            Redemption::Reused(Some("f1".into()))
        );
        store.purge(60 + TOMBSTONE_SECONDS);
        assert_eq!(store.redeem_code("h", 2), Redemption::Unknown);
    }

    #[test]
    fn rotation_reuse_and_revocation() {
        let store = MemoryStore::new(Capacity::DEFAULT);
        let k = key("s", &["a"]);
        let grant = store.upsert_grant(k.clone(), "l".into(), 0).unwrap();
        store.insert_family(family("f", k.clone(), "t1")).unwrap();
        assert!(matches!(
            store.rotate("t1", "t2".into(), 1),
            Rotation::Rotated(_)
        ));
        assert!(store.family_by_token("t2", 1).is_some());
        assert!(store.family_by_token("t1", 1).is_none());
        // Reusing t1 revokes the family, so t2 dies too.
        assert_eq!(store.rotate("t1", "t3".into(), 1), Rotation::Reused);
        assert_eq!(store.rotate("t2", "t4".into(), 1), Rotation::Invalid);
        // Revoking a grant drops its families; other subjects can't revoke it.
        store.insert_family(family("g", k, "u1")).unwrap();
        assert_eq!(store.revoke_grant("intruder", &grant.id), None);
        assert!(store.family_by_token("u1", 1).is_some());
        assert_eq!(store.revoke_grant("s", &grant.id), Some(grant));
        assert_eq!(store.rotate("u1", "u2".into(), 1), Rotation::Invalid);
        assert_eq!(store.grants_for("s"), Vec::<Grant>::new());
    }

    /// Regression from the proptest shrink (task 5.5): revoking a stale
    /// grant ID never touches a newer grant for the same key.
    #[test]
    fn revoking_a_stale_grant_id_spares_the_regranted_key() {
        let store = MemoryStore::new(Capacity::DEFAULT);
        let k = key("s", &["a"]);
        let old = store.upsert_grant(k.clone(), "l".into(), 0).unwrap();
        assert!(store.revoke_grant("s", &old.id).is_some());
        let new = store.upsert_grant(k.clone(), "l".into(), 1).unwrap();
        assert_ne!(old.id, new.id);
        store.insert_family(family("f", k, "t1")).unwrap();
        assert_eq!(store.revoke_grant("s", &old.id), None);
        assert!(store.family_by_token("t1", 1).is_some());
        assert_eq!(store.grants_for("s"), [new]);
    }

    #[test]
    fn concurrent_access_is_consistent() {
        let store = std::sync::Arc::new(MemoryStore::new(Capacity::DEFAULT));
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let store = std::sync::Arc::clone(&store);
                std::thread::spawn(move || {
                    for i in 0..200 {
                        store
                            .insert_pending(format!("{t}-{i}"), pending(100))
                            .unwrap();
                        store.upsert_grant(key("s", &[]), "l".into(), 0).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(lock(&store.pending).len(), 1600);
        assert_eq!(store.grants_for("s").len(), 1);
    }
}
